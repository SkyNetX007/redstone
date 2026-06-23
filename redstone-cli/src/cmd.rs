// redstone-cli/src/cmd.rs
use crate::{Cli, ConfigAction, InitType, Shell};
use clap::CommandFactory;
use rust_i18n::t;
use std::io::Write;
use std::path::Path;

// ─── spawn_background ───

fn spawn_background(exe: &Path, yaml_path: &str) -> Result<std::process::Child, std::io::Error> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let mut cmd = std::process::Command::new(exe);
        cmd.arg("_daemon")
            .arg(yaml_path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        cmd.spawn()
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new(exe)
            .arg("_daemon")
            .arg(yaml_path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .creation_flags(0x08000000)
            .spawn()
    }
}

// ─── Helpers ───

fn confirm_action(prompt: &str) -> bool {
    print!("{} ", prompt);
    let _ = std::io::stdout().flush();
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).ok();
    matches!(input.trim().to_lowercase().as_str(), "y" | "yes")
}

// ─── Start ───

pub async fn start_cmd(profile_name: &str) {
    let path = match redstone_core::profile::find_profile(profile_name) {
        Ok(p) => p,
        Err(_) => {
            eprintln!("{}", t!("app.cli.profile_not_found", name = profile_name));
            return;
        }
    };

    let profile = match redstone_core::profile::ResolvedProfile::load(&path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!(
                "{}",
                t!(
                    "app.cli.start.load_err",
                    profile = profile_name,
                    error = e.to_string()
                )
            );
            return;
        }
    };

    if redstone_core::profile::validate_profile_name(&profile.inner.name).is_err() {
        eprintln!(
            "{}",
            t!("app.cli.invalid_profile_name", name = &profile.inner.name)
        );
        return;
    }

    let name = &profile.inner.name;
    let canonical_yaml = redstone_core::profile::profile_yaml_path(name);

    if path != canonical_yaml {
        if let Some(parent) = canonical_yaml.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = profile.write_canonical(&canonical_yaml) {
            eprintln!("{}", t!("app.cli.start.import_err", error = e.to_string()));
            return;
        }
        println!(
            "{}",
            t!("app.cli.start.imported", path = canonical_yaml.display())
        );
    }

    let log_dir = redstone_core::profile::profile_log_dir(name);
    if let Err(e) = std::fs::create_dir_all(&log_dir) {
        eprintln!("{}", t!("app.cli.start.log_dir_err", error = e.to_string()));
        return;
    }

    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("{}", t!("app.cli.start.exe_err", error = e.to_string()));
            return;
        }
    };

    let daemon_path = canonical_yaml.to_string_lossy().into_owned();

    match spawn_background(&exe, &daemon_path) {
        Ok(_) => {
            println!("{}", t!("app.cli.start.ok", name = name));
            println!("{}", t!("app.cli.start.hint_status", name = name));
            println!("{}", t!("app.cli.start.hint_attach", name = name));
        }
        Err(e) => {
            eprintln!("{}", t!("app.cli.start.start_err", error = e.to_string()));
        }
    }
}

// ─── Stop ───

pub async fn stop_cmd(profile_name: &str, wait: bool, timeout: Option<u64>) {
    let mut client = match redstone_core::ipc::DaemonClient::connect(profile_name).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "{}",
                t!(
                    "app.cli.connect_err",
                    profile = profile_name,
                    error = e.to_string()
                )
            );
            return;
        }
    };
    let resolved = redstone_core::profile::resolve_profile_name(profile_name);
    println!("{}", t!("app.cli.stop.sending", profile = &resolved));
    let _ = client.write_stdin("stop\n").await;

    if wait {
        let timeout_secs = timeout.unwrap_or(30);
        let resolved = redstone_core::profile::resolve_profile_name(profile_name);
        println!("{}", t!("app.cli.stop.wait", timeout = timeout_secs));
        let start = std::time::Instant::now();
        let delay = std::time::Duration::from_millis(250);
        loop {
            if start.elapsed().as_secs() >= timeout_secs {
                println!("{}", t!("app.cli.stop.timeout", timeout = timeout_secs));
                let _ = client.kill().await;
                break;
            }
            if let Ok(Some(state)) = redstone_core::profile::read_server_state(&resolved)
                && !state.running
            {
                println!("{}", t!("app.cli.stop.ok", profile = &resolved));
                break;
            }
            tokio::time::sleep(delay).await;
        }
    }
}

// ─── Kill ───

pub async fn kill_cmd(profile_name: &str) {
    let mut client = match redstone_core::ipc::DaemonClient::connect(profile_name).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "{}",
                t!(
                    "app.cli.connect_err",
                    profile = profile_name,
                    error = e.to_string()
                )
            );
            return;
        }
    };
    let resolved = redstone_core::profile::resolve_profile_name(profile_name);
    if !confirm_action(&t!("app.cli.kill.confirm", name = &resolved)) {
        println!("{}", t!("app.cli.action_cancelled"));
        return;
    }
    println!("{}", t!("app.cli.kill.ok", profile = &resolved));
    let _ = client.kill().await;
}

// ─── Restart ───

pub async fn restart_cmd(profile_name: &str) {
    if let Ok(mut client) = redstone_core::ipc::DaemonClient::connect(profile_name).await {
        let resolved = redstone_core::profile::resolve_profile_name(profile_name);
        println!("{}", t!("app.cli.restart.stopping", profile = &resolved));
        let _ = client.write_stdin("stop\n").await;
        drop(client);
        let resolved = redstone_core::profile::resolve_profile_name(profile_name);
        let start = std::time::Instant::now();
        loop {
            if start.elapsed().as_secs() > 60 {
                break;
            }
            if let Ok(Some(state)) = redstone_core::profile::read_server_state(&resolved)
                && !state.running
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    }
    start_cmd(profile_name).await;
}

// ─── Status ───

pub async fn status_cmd(profile_name: &str) {
    let resolved = redstone_core::profile::resolve_profile_name(profile_name);

    if let Ok(mut client) = redstone_core::ipc::DaemonClient::connect(profile_name).await {
        match client.query_status().await {
            Ok(resp) => {
                if let redstone_core::ipc::DaemonResponse::StatusResp {
                    pid,
                    running,
                    uptime_secs,
                } = resp
                {
                    println!("{}", t!("app.cli.status.header", profile = &resolved));
                    println!("{}", t!("app.cli.status.pid", pid = pid));
                    println!("{}", t!("app.cli.status.running", running = running));
                    println!("{}", t!("app.cli.status.uptime", uptime = uptime_secs));
                }
            }
            Err(e) => println!("{}", t!("app.cli.status.failed", error = e.to_string())),
        }
    }

    if let Ok(yaml_path) = redstone_core::profile::find_profile(profile_name)
        && let Ok(profile) = redstone_core::profile::ResolvedProfile::load(&yaml_path)
    {
        let port = profile.inner.port;
        match redstone_core::slp::ping_server("127.0.0.1", port).await {
            Ok(s) => {
                println!("{}", t!("app.cli.status.version", version = s.version));
                println!(
                    "{}",
                    t!(
                        "app.cli.status.players",
                        online = s.online_players,
                        max = s.max_players
                    )
                );
                println!("{}", t!("app.cli.status.motd", motd = s.motd));
                println!("{}", t!("app.cli.status.latency", latency = s.latency_ms));
            }
            Err(_) => println!("{}", t!("app.cli.status.unreachable", port = port)),
        }
    }
}

// ─── Attach ───

pub async fn attach_cmd(profile_name: &str) {
    let mut client = match redstone_core::ipc::DaemonClient::connect(profile_name).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "{}",
                t!(
                    "app.cli.connect_err",
                    profile = profile_name,
                    error = e.to_string()
                )
            );
            return;
        }
    };

    use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
    if let Err(e) = enable_raw_mode() {
        eprintln!(
            "{}",
            t!("app.cli.attach.raw_mode_err", error = e.to_string())
        );
        return;
    }

    let resolved = redstone_core::profile::resolve_profile_name(profile_name);
    println!("{}", t!("app.cli.attach.ok", profile = &resolved));

    let result = run_attach_loop(&mut client).await;

    let _ = disable_raw_mode();
    if let Err(e) = result {
        eprintln!("{}", t!("app.cli.attach.err", error = e.to_string()));
    }
}

async fn run_attach_loop(
    client: &mut redstone_core::ipc::DaemonClient,
) -> Result<(), Box<dyn std::error::Error>> {
    use crossterm::event::{self, Event, KeyCode, KeyModifiers};
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Event>();

    std::thread::spawn(move || {
        while let Ok(ev) = event::read() {
            if tx.send(ev).is_err() {
                break;
            }
        }
    });

    loop {
        tokio::select! {
            Some(ev) = rx.recv() => {
                match ev {
                    Event::Key(key) => {
                        if key.code == KeyCode::Char('q') && key.modifiers == KeyModifiers::CONTROL {
                            break;
                        }
                        if key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL {
                            break;
                        }
                        let data = match key.code {
                            KeyCode::Enter => "\n".to_string(),
                            KeyCode::Backspace => "\x7f".to_string(),
                            KeyCode::Tab => "\t".to_string(),
                            KeyCode::Char(c) => c.to_string(),
                            _ => continue,
                        };
                        let _ = client.write_stdin(&data).await;
                    }
                    _ => break,
                }
            }
            r = client.read_response() => {
                match r {
                    Ok(resp) => match resp {
                        redstone_core::ipc::DaemonResponse::Stdout { data } => println!("{}", data),
                        redstone_core::ipc::DaemonResponse::Stderr { data } => eprintln!("{}", data),
                        redstone_core::ipc::DaemonResponse::Exited { status } => {
                            println!("{}", t!("app.cli.attach.server_exited", status = status));
                            break;
                        }
                        redstone_core::ipc::DaemonResponse::Error { message } => {
                            eprintln!("{}", t!("app.cli.attach.daemon_err", msg = message));
                            break;
                        }
                        _ => {}
                    },
                    Err(e) => {
                        eprintln!("{}", t!("app.cli.attach.conn_lost", error = e.to_string()));
                        break;
                    }
                }
            }
        }
    }

    Ok(())
}

// ─── List ───

pub async fn list_cmd(online_only: bool, offline_only: bool) {
    let profiles = redstone_core::profile::list_all_profiles();
    if profiles.is_empty() {
        println!("{}", t!("app.cli.list.none"));
        return;
    }

    println!(
        "{:12}  {:<8}  {}",
        t!("app.cli.list.name_header"),
        t!("app.cli.list.status_header"),
        t!("app.cli.list.info_header")
    );
    println!("{}", "-".repeat(42));

    for entry in &profiles {
        let state = redstone_core::profile::read_server_state(&entry.name)
            .ok()
            .flatten();
        let running = state.as_ref().is_some_and(|s| s.running);

        if online_only && !running {
            continue;
        }
        if offline_only && running {
            continue;
        }

        print!("{:12}  ", entry.name);
        if let Some(s) = state {
            if s.running {
                let uptime = s
                    .started_at
                    .map(|t| {
                        let now = redstone_core::profile::now_epoch();
                        let elapsed = now.saturating_sub(t);
                        format!("{}h {}m", elapsed / 3600, (elapsed % 3600) / 60)
                    })
                    .unwrap_or_default();
                println!(
                    "{}  PID {}  {}",
                    t!("app.cli.list.online_label"),
                    s.pid.unwrap_or(0),
                    uptime
                );
            } else {
                println!("{}", t!("app.cli.list.offline_label"));
            }
        } else {
            println!("{}", t!("app.cli.list.offline_label"));
        }
    }
}

// ─── Rm ───

pub async fn rm_cmd(profile_name: &str, force: bool) {
    if redstone_core::profile::validate_profile_name(profile_name).is_err() {
        eprintln!(
            "{}",
            t!("app.cli.invalid_profile_name", name = profile_name)
        );
        return;
    }

    let yaml = redstone_core::profile::profile_yaml_path(profile_name);
    let data_dir = redstone_core::profile::profile_data_dir(profile_name);

    if !yaml.exists() && !data_dir.exists() {
        eprintln!("{}", t!("app.cli.profile_not_found", name = profile_name));
        return;
    }

    if let Ok(Some(state)) = redstone_core::profile::read_server_state(profile_name)
        && state.running
    {
        if !force {
            eprintln!("{}", t!("app.cli.rm.running", name = profile_name));
            return;
        }
        kill_cmd(profile_name).await;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    if !force && !confirm_action(&t!("app.cli.rm.confirm", name = profile_name)) {
        println!("{}", t!("app.cli.action_cancelled"));
        return;
    }

    if yaml.exists()
        && let Err(e) = std::fs::remove_file(&yaml)
    {
        eprintln!(
            "{}",
            t!("app.cli.rm.delete_err", path = yaml.display(), error = e)
        );
    }

    if data_dir.exists()
        && let Err(e) = std::fs::remove_dir_all(&data_dir)
    {
        eprintln!(
            "{}",
            t!(
                "app.cli.rm.delete_err",
                path = data_dir.display(),
                error = e
            )
        );
    }

    println!("{}", t!("app.cli.rm.ok", name = profile_name));
}

// ─── Rename ───

pub async fn rename_cmd(from: &str, to: &str) {
    if redstone_core::profile::validate_profile_name(from).is_err() {
        eprintln!("{}", t!("app.cli.invalid_profile_name", name = from));
        return;
    }
    if redstone_core::profile::validate_profile_name(to).is_err() {
        eprintln!("{}", t!("app.cli.invalid_profile_name", name = to));
        return;
    }

    let old_yaml = redstone_core::profile::profile_yaml_path(from);
    if !old_yaml.exists() {
        eprintln!("{}", t!("app.cli.profile_not_found", name = from));
        return;
    }

    let new_yaml = redstone_core::profile::profile_yaml_path(to);
    if new_yaml.exists() {
        eprintln!("{}", t!("app.cli.rename.exists", name = to));
        return;
    }

    if let Ok(Some(state)) = redstone_core::profile::read_server_state(from)
        && state.running
    {
        eprintln!("{}", t!("app.cli.rename.running", name = from));
        return;
    }

    let content = match std::fs::read_to_string(&old_yaml) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{}", e);
            return;
        }
    };

    let mut profile: redstone_core::profile::Profile = match serde_yaml::from_str(&content) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}", e);
            return;
        }
    };

    profile.name = to.to_string();

    let yaml = match serde_yaml::to_string(&profile) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}", e);
            return;
        }
    };
    if let Err(e) = std::fs::write(&new_yaml, yaml) {
        eprintln!("{}", e);
        return;
    }

    let old_dir = redstone_core::profile::profile_data_dir(from);
    if old_dir.exists() {
        let new_dir = redstone_core::profile::profile_data_dir(to);
        if let Err(e) = std::fs::rename(&old_dir, &new_dir) {
            let _ = std::fs::remove_file(&new_yaml);
            eprintln!("{}", e);
            return;
        }
    }

    let _ = std::fs::remove_file(&old_yaml);

    println!("{}", t!("app.cli.rename.ok", old = from, new = to));
}

// ─── Log ───

pub async fn log_cmd(profile_name: &str, follow: bool) {
    let resolved = redstone_core::profile::resolve_profile_name(profile_name);
    let log_path = redstone_core::profile::profile_log_path(&resolved);

    if !log_path.exists() {
        println!("{}", t!("app.cli.log.none", profile = &resolved));
        return;
    }

    if follow {
        let mut last_size = std::fs::metadata(&log_path).map(|m| m.len()).unwrap_or(0);
        loop {
            if let Ok(current_size) = std::fs::metadata(&log_path).map(|m| m.len())
                && current_size > last_size
            {
                let content = std::fs::read_to_string(&log_path).unwrap_or_default();
                print!("{}", &content[last_size as usize..]);
                let _ = std::io::stdout().flush();
                last_size = current_size;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    } else {
        match std::fs::read_to_string(&log_path) {
            Ok(content) => print!("{}", content),
            Err(e) => eprintln!("{}", t!("app.cli.log.err", error = e.to_string())),
        }
    }
}

// ─── Completion ───

pub fn completion_cmd(shell: Shell) {
    let shell_type = match shell {
        Shell::Bash => clap_complete::Shell::Bash,
        Shell::Fish => clap_complete::Shell::Fish,
        Shell::Zsh => clap_complete::Shell::Zsh,
    };
    let mut cmd = Cli::command().about(t!("app.about").to_string());
    let mut buf = std::io::stdout();
    clap_complete::generate(shell_type, &mut cmd, "redstone", &mut buf);
}

// ─── Init ───

pub fn init_cmd(server_type: InitType, output: Option<String>) {
    let profile = match server_type {
        InitType::Minecraft => redstone_core::profile::Profile {
            name: "minecraft".to_string(),
            command: None,
            args: None,
            server_dir: None,
            jar: "server.jar".to_string(),
            memory: Some(redstone_core::profile::MemoryConfig {
                min: "2G".to_string(),
                max: "4G".to_string(),
            }),
            jvm_args: None,
            auto_restart: Some(false),
            auto_restart_delay: None,
            port: 25565,
        },
        InitType::Cmd => redstone_core::profile::Profile {
            name: "cmd".to_string(),
            command: Some("./server".to_string()),
            args: Some(vec![]),
            server_dir: None,
            jar: "server.jar".to_string(),
            memory: None,
            jvm_args: None,
            auto_restart: Some(false),
            auto_restart_delay: None,
            port: 25565,
        },
    };

    let yaml = match serde_yaml::to_string(&profile) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}", e);
            return;
        }
    };

    match output {
        Some(path) => {
            let path = std::path::Path::new(&path);
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if std::fs::write(path, yaml.as_bytes()).is_ok() {
                println!("{}", t!("app.cli.init.ok", path = path.display()));
            } else {
                eprintln!("{}", t!("app.cli.init.write_err", path = path.display()));
            }
        }
        None => println!("{}", yaml),
    }
}

// ─── Exec ───

pub async fn exec_cmd(profile_name: &str, command: &str) {
    let mut client = match redstone_core::ipc::DaemonClient::connect(profile_name).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "{}",
                t!(
                    "app.cli.connect_err",
                    profile = profile_name,
                    error = e.to_string()
                )
            );
            return;
        }
    };
    let resolved = redstone_core::profile::resolve_profile_name(profile_name);
    let _ = client.write_stdin(&format!("{}\n", command)).await;
    println!("{}", t!("app.cli.exec.ok", profile = &resolved));
}

// ─── Config ───

fn global_config_path() -> std::path::PathBuf {
    redstone_core::profile::redstone_dir().join("config.yaml")
}

pub async fn config_cmd(profile: Option<&str>, action: ConfigAction) {
    let path = match profile {
        Some(name) => {
            let yaml = redstone_core::profile::profile_yaml_path(name);
            if !yaml.exists() {
                eprintln!("{}", t!("app.cli.profile_not_found", name = name));
                return;
            }
            yaml
        }
        None => global_config_path(),
    };

    match action {
        ConfigAction::Get { key } => config_get(&path, key),
        ConfigAction::Set { key, value } => config_set(&path, &key, &value),
    }
}

fn config_get(path: &Path, key: Option<String>) {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => {
            println!("{}", t!("app.cli.config.empty"));
            return;
        }
    };

    match key {
        Some(k) => {
            let value: serde_yaml::Value = match serde_yaml::from_str(&content) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("{}", t!("app.cli.config.parse_err", error = e.to_string()));
                    return;
                }
            };
            match resolve_yaml_key(&value, &k) {
                Some(v) => print_yaml_value(v),
                None => eprintln!("{}", t!("app.cli.config.key_not_found", key = k)),
            }
        }
        None => println!("{}", content),
    }
}

fn config_set(path: &Path, key: &str, value: &str) {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => String::from("{}"),
    };

    let mut doc: serde_yaml::Value = match serde_yaml::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{}", t!("app.cli.config.parse_err", error = e.to_string()));
            return;
        }
    };

    if !doc.is_mapping() {
        doc = serde_yaml::Value::Mapping(serde_yaml::Mapping::new());
    }

    let parsed = parse_yaml_value(value);
    set_nested(&mut doc, key, parsed);

    let output = match serde_yaml::to_string(&doc) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "{}",
                t!("app.cli.config.serialize_err", error = e.to_string())
            );
            return;
        }
    };

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(path, output.as_bytes()) {
        eprintln!("{}", t!("app.cli.config.write_err", error = e.to_string()));
        return;
    }

    println!("{}", t!("app.cli.config.ok", key = key, value = value));
}

// ─── YAML helpers ───

fn resolve_yaml_key<'a>(value: &'a serde_yaml::Value, key: &str) -> Option<&'a serde_yaml::Value> {
    let parts: Vec<&str> = key.split('.').collect();
    let mut current = value;
    for part in parts {
        current = current.get(part)?;
    }
    Some(current)
}

fn print_yaml_value(value: &serde_yaml::Value) {
    match value {
        serde_yaml::Value::String(s) => println!("{}", s),
        serde_yaml::Value::Number(n) => println!("{}", n),
        serde_yaml::Value::Bool(b) => println!("{}", b),
        serde_yaml::Value::Null => println!("{}", t!("app.cli.config.null_value")),
        other => {
            if let Ok(s) = serde_yaml::to_string(other) {
                print!("{}", s);
            }
        }
    }
}

fn parse_yaml_value(s: &str) -> serde_yaml::Value {
    if let Ok(b) = s.parse::<bool>() {
        return serde_yaml::Value::Bool(b);
    }
    if let Ok(i) = s.parse::<i64>() {
        return serde_yaml::Value::Number(serde_yaml::Number::from(i));
    }

    serde_yaml::Value::String(s.to_string())
}

fn set_nested(value: &mut serde_yaml::Value, key: &str, val: serde_yaml::Value) {
    if let Some(dot) = key.find('.') {
        let parent = &key[..dot];
        let child = &key[dot + 1..];
        if !value[parent].is_mapping() {
            value[parent] = serde_yaml::Value::Mapping(serde_yaml::Mapping::new());
        }
        set_nested(&mut value[parent], child, val);
    } else {
        value[key] = val;
    }
}

// ─── Internal Daemon ───

pub async fn _daemon_cmd(yaml_path: &str) {
    let path = Path::new(yaml_path);
    let profile = match redstone_core::profile::ResolvedProfile::load(path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!(
                "{}",
                t!("app.cli.start.daemon_load_err", error = e.to_string())
            );
            return;
        }
    };
    if let Err(e) = redstone_core::ipc::run_daemon(profile).await {
        eprintln!(
            "{}",
            t!("app.cli.start.daemon_run_err", error = e.to_string())
        );
    }
}
