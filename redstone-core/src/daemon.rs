use std::path::Path;
use std::process::{Child, Command, Stdio};

#[cfg(unix)]
use std::os::unix::process::CommandExt;
#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// Spawn the daemon process for a profile.
/// Returns the child handle and a pipe to its stderr.
pub fn spawn_daemon(
    exe: &Path,
    yaml_path: &str,
) -> Result<(Child, std::process::ChildStderr), std::io::Error> {
    let mut cmd = Command::new(exe);
    cmd.arg("_daemon")
        .arg(yaml_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    #[cfg(unix)]
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }

    #[cfg(windows)]
    {
        cmd.creation_flags(0x08000000);
    }

    let mut child = cmd.spawn()?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| std::io::Error::other("failed to capture stderr"))?;
    Ok((child, stderr))
}

/// Check if a daemon is already running by connecting to its IPC socket.
/// Returns immediately — connect to a non-existent abstract socket
/// fails instantly with ENOENT on Linux.
pub async fn check_daemon_alive(name: &str) -> bool {
    crate::ipc::DaemonClient::connect(name).await.is_ok()
}

/// Try to kill a server: first via daemon IPC, fallback to orphan process scan.
pub async fn kill_server(profile_name: &str) -> Result<(), String> {
    if let Ok(mut client) = crate::ipc::DaemonClient::connect(profile_name).await {
        client.kill().await.map_err(|e| e.to_string())?;
        return Ok(());
    }
    kill_orphan_processes(profile_name).await
}

#[cfg(unix)]
async fn kill_orphan_processes(name: &str) -> Result<(), String> {
    let pattern = format!("[redstone] {} (", name);
    let mut pids: Vec<i32> = Vec::new();

    let proc = match std::fs::read_dir("/proc") {
        Ok(d) => d,
        Err(_) => return Err("Cannot access /proc".to_string()),
    };

    for entry in proc {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let pid_str = entry.file_name();
        let pid: i32 = match pid_str.to_str().and_then(|s| s.parse().ok()) {
            Some(p) => p,
            None => continue,
        };
        let cmdline_path = entry.path().join("cmdline");
        let Ok(cmdline) = std::fs::read_to_string(&cmdline_path) else {
            continue;
        };
        if cmdline.contains(&pattern) {
            pids.push(pid);
        }
    }

    let port = crate::profile::read_server_state(name)
        .ok()
        .flatten()
        .map(|s| s.port)
        .unwrap_or(25565);

    if !pids.is_empty() {
        // SIGTERM first
        for &pid in &pids {
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
        }

        // Wait for graceful exit
        for _ in 0..20 {
            pids.retain(|&pid| unsafe { libc::kill(pid, 0) == 0 });
            if pids.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }

        // SIGKILL survivors
        for &pid in &pids {
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
    }

    crate::profile::write_server_state(
        name,
        &crate::profile::ServerState {
            name: name.to_string(),
            pid: None,
            running: false,
            started_at: None,
            stopped_at: Some(crate::profile::now_epoch()),
            port,
        },
    )
    .map_err(|e| e.to_string())
}

#[cfg(windows)]
async fn kill_orphan_processes(name: &str) -> Result<(), String> {
    let yaml_path = crate::profile::profile_yaml_path(name);
    let exe_name = if let Ok(profile) = crate::profile::ResolvedProfile::load(&yaml_path) {
        // Native command: can safely taskkill by IM name
        if profile.inner.command.is_some() {
            profile.inner.command.as_deref()
        } else {
            // Java: killing all java.exe is too dangerous
            None
        }
    } else {
        None
    };

    if let Some(exe) = exe_name {
        let output = std::process::Command::new("taskkill")
            .args(&["/IM", exe, "/F"])
            .output()
            .map_err(|e| format!("Failed to run taskkill: {}", e))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("taskkill failed: {}", stderr));
        }
    } else {
        return Err(format!(
            "Cannot kill orphan server '{}' on Windows: Java JVM process is ambiguous. \
             Please kill it manually via Task Manager.",
            name
        ));
    }

    let port = crate::profile::read_server_state(name)
        .ok()
        .flatten()
        .map(|s| s.port)
        .unwrap_or(25565);

    crate::profile::write_server_state(
        name,
        &crate::profile::ServerState {
            name: name.to_string(),
            pid: None,
            running: false,
            started_at: None,
            stopped_at: Some(crate::profile::now_epoch()),
            port,
        },
    )
    .map_err(|e| e.to_string())
}

/// Wait for the daemon to be ready by polling its IPC socket.
/// Returns Ok(()) once a connection succeeds, or an error after timeout.
pub async fn verify_daemon_ready(name: &str, timeout: std::time::Duration) -> Result<(), String> {
    use tokio::time::sleep;
    let start = std::time::Instant::now();
    loop {
        match crate::ipc::DaemonClient::connect(name).await {
            Ok(_) => return Ok(()),
            Err(_) => {
                if start.elapsed() >= timeout {
                    return Err(format!(
                        "Daemon for '{}' did not become ready within {:?}",
                        name, timeout
                    ));
                }
                sleep(std::time::Duration::from_millis(200)).await;
            }
        }
    }
}
