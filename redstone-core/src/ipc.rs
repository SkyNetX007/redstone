// redstone-core/src/ipc.rs
use crate::profile::ResolvedProfile;
use interprocess::local_socket::ConnectOptions;
use interprocess::local_socket::tokio::Stream as LocalSocketStream;
use interprocess::local_socket::traits::tokio::Listener as _;
use interprocess::local_socket::traits::tokio::Stream as _;
use interprocess::local_socket::{GenericNamespaced, ListenerOptions, Name, ToNsName};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::ChildStdin;
use tokio::sync::broadcast;

// ─── IPC Protocol Messages ───

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DaemonRequest {
    #[serde(rename = "stdin")]
    Stdin { data: String },
    #[serde(rename = "subscribe")]
    Subscribe,
    #[serde(rename = "unsubscribe")]
    Unsubscribe,
    #[serde(rename = "status")]
    Status,
    #[serde(rename = "kill")]
    Kill,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DaemonResponse {
    #[serde(rename = "stdout")]
    Stdout { data: String },
    #[serde(rename = "stderr")]
    Stderr { data: String },
    #[serde(rename = "status_resp")]
    StatusResp {
        pid: u32,
        running: bool,
        uptime_secs: u64,
    },
    #[serde(rename = "exited")]
    Exited { status: i32 },
    #[serde(rename = "error")]
    Error { message: String },
}

// ─── Socket Name ───

pub fn daemon_socket_name(profile_name: &str) -> String {
    format!("redstone-{}", profile_name)
}

pub fn build_name(profile_name: &str) -> std::io::Result<Name<'static>> {
    daemon_socket_name(profile_name)
        .to_ns_name::<GenericNamespaced>()
        .map(|n| n.into_owned())
}

// ─── Daemon ───

pub async fn run_daemon(profile: ResolvedProfile) -> Result<(), Box<dyn std::error::Error>> {
    let name_str = profile.inner.name.clone();
    crate::profile::validate_profile_name(&name_str)?;
    let sock_name = build_name(&name_str)?;
    let auto_restart = profile.inner.auto_restart.unwrap_or(false);
    let restart_delay =
        std::time::Duration::from_secs(profile.inner.auto_restart_delay.unwrap_or(5));
    let port = profile.inner.port;

    let log_dir = crate::profile::profile_log_dir(&name_str);
    std::fs::create_dir_all(&log_dir)?;
    let log_path = crate::profile::profile_log_path(&name_str);

    let listener = match ListenerOptions::new().name(sock_name).create_tokio() {
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            return Err("Socket already in use (server already running?)".into());
        }
        x => x?,
    };

    let killed = Arc::new(AtomicBool::new(false));

    loop {
        let mut cmd = profile.build_java_command();
        let mut child = cmd.spawn()?;
        let child_stdin = child.stdin.take().ok_or("failed to capture stdin")?;
        let child_stdout = child.stdout.take().ok_or("failed to capture stdout")?;
        let child_stderr = child.stderr.take().ok_or("failed to capture stderr")?;

        let pid = child.id().unwrap_or(0);

        crate::profile::write_server_state(
            &name_str,
            &crate::profile::ServerState {
                name: name_str.clone(),
                pid: Some(pid),
                running: true,
                started_at: Some(crate::profile::now_epoch()),
                stopped_at: None,
                port,
            },
        )?;

        let running = Arc::new(AtomicBool::new(true));
        let (stdout_tx, _) = broadcast::channel::<String>(4096);
        let (stderr_tx, _) = broadcast::channel::<String>(4096);
        let start_time = std::time::Instant::now();

        let stx = stdout_tx.clone();
        let run = running.clone();
        let log_p = log_path.clone();
        tokio::spawn(async move {
            let mut log_f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_p)
                .ok();
            let mut reader = BufReader::new(child_stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if let Some(ref mut f) = log_f {
                    let _ = writeln!(f, "{}", line);
                }
                if line.contains("Done") {
                    let msg = format!("✨ [Core] Server started! PID: {}", pid);
                    if let Some(ref mut f) = log_f {
                        let _ = writeln!(f, "{}", msg);
                    }
                    let _ = stx.send(msg);
                }
                let _ = stx.send(line);
            }
            run.store(false, Ordering::Relaxed);
        });

        let stx = stderr_tx.clone();
        let log_p = log_path.clone();
        tokio::spawn(async move {
            let mut log_f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_p)
                .ok();
            let mut reader = BufReader::new(child_stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if let Some(ref mut f) = log_f {
                    let _ = writeln!(f, "{}", line);
                }
                let _ = stx.send(line);
            }
        });

        let stdin_ref = Arc::new(tokio::sync::Mutex::new(child_stdin));

        while running.load(Ordering::Relaxed) {
            if killed.load(Ordering::Relaxed) {
                break;
            }
            tokio::select! {
                conn = tokio::time::timeout(
                    std::time::Duration::from_secs(1),
                    listener.accept(),
                ) => {
                    if let Ok(Ok(conn)) = conn {
                        let stdout_rx = stdout_tx.subscribe();
                        let stderr_rx = stderr_tx.subscribe();
                        let stdin_clone = stdin_ref.clone();
                        let start = start_time;
                        let run = running.clone();
                        let kill = killed.clone();
                        tokio::spawn(async move {
                            handle_client(conn, stdin_clone, stdout_rx, stderr_rx, pid, start, run, kill).await;
                        });
                    }
                }
                _ = tokio::signal::ctrl_c() => {
                    killed.store(true, Ordering::Relaxed);
                    break;
                }
            }
        }

        running.store(false, Ordering::Relaxed);
        let _ = child.kill().await;
        let exit_status = child.wait().await?;
        let exit_code = exit_status.code().unwrap_or(-1);

        if killed.load(Ordering::Relaxed) || exit_code == 0 || !auto_restart {
            break;
        }

        tokio::time::sleep(restart_delay).await;
    }

    let now = crate::profile::now_epoch();
    crate::profile::write_server_state(
        &name_str,
        &crate::profile::ServerState {
            name: name_str.clone(),
            pid: None,
            running: false,
            started_at: None,
            stopped_at: Some(now),
            port,
        },
    )?;

    Ok(())
}

async fn handle_client(
    conn: LocalSocketStream,
    stdin: Arc<tokio::sync::Mutex<ChildStdin>>,
    mut stdout_rx: broadcast::Receiver<String>,
    mut stderr_rx: broadcast::Receiver<String>,
    pid: u32,
    start_time: std::time::Instant,
    running: Arc<AtomicBool>,
    killed: Arc<AtomicBool>,
) {
    let (reader, mut writer) = conn.split();
    let mut buf_reader = BufReader::new(reader);
    let mut line_buf = String::new();
    let mut subscribed = false;

    loop {
        tokio::select! {
            r = buf_reader.read_line(&mut line_buf) => {
                match r {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }

                let trimmed = line_buf.trim();
                let req: DaemonRequest = match serde_json::from_str(trimmed) {
                    Ok(r) => r,
                    Err(_) => {
                        line_buf.clear();
                        continue;
                    }
                };
                line_buf.clear();

                match req {
                    DaemonRequest::Stdin { data } => {
                        let mut guard = stdin.lock().await;
                        let _ = guard.write_all(data.as_bytes()).await;
                        let _ = guard.flush().await;
                    }
                    DaemonRequest::Subscribe => subscribed = true,
                    DaemonRequest::Unsubscribe => subscribed = false,
                    DaemonRequest::Status => {
                        let resp = DaemonResponse::StatusResp {
                            pid,
                            running: running.load(Ordering::Relaxed),
                            uptime_secs: start_time.elapsed().as_secs(),
                        };
                        let Ok(mut json) = serde_json::to_string(&resp) else { break };
                        json.push('\n');
                        let _ = writer.write_all(json.as_bytes()).await;
                    }
                    DaemonRequest::Kill => {
                        killed.store(true, Ordering::Relaxed);
                        running.store(false, Ordering::Relaxed);
                        break;
                    }
                }
            }
            r = stdout_rx.recv(), if subscribed => {
                if let Ok(line) = r {
                    let resp = DaemonResponse::Stdout { data: line };
                    let Ok(mut json) = serde_json::to_string(&resp) else { break };
                    json.push('\n');
                    if writer.write_all(json.as_bytes()).await.is_err() {
                        break;
                    }
                }
            }
            r = stderr_rx.recv(), if subscribed => {
                if let Ok(line) = r {
                    let resp = DaemonResponse::Stderr { data: line };
                    let Ok(mut json) = serde_json::to_string(&resp) else { break };
                    json.push('\n');
                    if writer.write_all(json.as_bytes()).await.is_err() {
                        break;
                    }
                }
            }
        }
    }
}

// ─── Client Helper ───

use interprocess::local_socket::tokio::RecvHalf;

pub struct DaemonClient {
    writer: interprocess::local_socket::tokio::SendHalf,
    reader: BufReader<RecvHalf>,
    buf: String,
}

impl DaemonClient {
    pub async fn connect(profile_name: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let resolved = crate::profile::resolve_profile_name(profile_name);
        let name = build_name(&resolved)?;
        let stream = match tokio::time::timeout(
            std::time::Duration::from_secs(2),
            ConnectOptions::new().name(name).connect_tokio(),
        )
        .await
        {
            Ok(s) => s?,
            Err(_) => return Err("Connection to daemon timed out (2s)".into()),
        };
        let (reader, writer) = stream.split();
        Ok(Self {
            writer,
            reader: BufReader::new(reader),
            buf: String::new(),
        })
    }

    pub async fn write_stdin(&mut self, data: &str) -> Result<(), Box<dyn std::error::Error>> {
        let req = DaemonRequest::Stdin {
            data: data.to_string(),
        };
        let mut json = serde_json::to_string(&req)?;
        json.push('\n');
        self.writer.write_all(json.as_bytes()).await?;
        self.writer.flush().await?;
        Ok(())
    }

    pub async fn query_status(&mut self) -> Result<DaemonResponse, Box<dyn std::error::Error>> {
        let req = DaemonRequest::Status;
        let mut json = serde_json::to_string(&req)?;
        json.push('\n');
        self.writer.write_all(json.as_bytes()).await?;
        self.writer.flush().await?;
        self.read_response().await
    }

    pub async fn kill(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let req = DaemonRequest::Kill;
        let mut json = serde_json::to_string(&req)?;
        json.push('\n');
        self.writer.write_all(json.as_bytes()).await?;
        self.writer.flush().await?;
        Ok(())
    }

    pub async fn read_response(&mut self) -> Result<DaemonResponse, Box<dyn std::error::Error>> {
        self.buf.clear();
        self.reader.read_line(&mut self.buf).await?;
        if self.buf.is_empty() {
            return Err("daemon connection closed".into());
        }
        let resp: DaemonResponse = serde_json::from_str(self.buf.trim())?;
        Ok(resp)
    }
}
