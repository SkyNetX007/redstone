// redstone-tui/src/state.rs
use crate::event::Event;
use ratatui::layout::Rect;
use std::collections::VecDeque;
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use tokio::sync::mpsc;

const MAX_CONSOLE_LINES: usize = 1000;

pub struct ConsoleBuffer {
    pub lines: VecDeque<String>,
    pub scroll_offset: usize,
}

impl ConsoleBuffer {
    fn new() -> Self {
        Self {
            lines: VecDeque::with_capacity(256),
            scroll_offset: 0,
        }
    }

    pub fn push(&mut self, line: String) {
        let cleaned = String::from_utf8(strip_ansi_escapes::strip(line.as_bytes())).unwrap_or(line);
        if self.lines.len() >= MAX_CONSOLE_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(cleaned);
    }

    pub fn scroll_up(&mut self) {
        let max_scroll = self.lines.len().saturating_sub(1);
        self.scroll_offset = self.scroll_offset.saturating_add(1).min(max_scroll);
    }

    pub fn scroll_down(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_sub(1);
    }

    pub fn scroll_top(&mut self) {
        self.scroll_offset = self.lines.len().saturating_sub(1);
    }

    pub fn scroll_bottom(&mut self) {
        self.scroll_offset = 0;
    }

    pub fn is_at_bottom(&self) -> bool {
        self.scroll_offset == 0
    }

    pub fn visible_lines(&self, height: usize) -> Vec<String> {
        let total = self.lines.len();
        if total == 0 {
            return Vec::new();
        }
        let offset = self.scroll_offset.min(total.saturating_sub(1));
        let start = total.saturating_sub(height).saturating_sub(offset);
        let end = total.saturating_sub(offset);
        self.lines
            .iter()
            .skip(start)
            .take(end.saturating_sub(start))
            .cloned()
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionStatus {
    Offline,
    Running,
}

pub struct ProfileState {
    pub name: String,
    pub status: ConnectionStatus,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Focus {
    ServerList,
    Console,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Overlay {
    None,
    ConfirmKill(String),
}

pub struct LayoutRects {
    pub server_list: Rect,
    pub console: Rect,
    pub status_panel: Rect,
}

pub struct State {
    pub profiles: Vec<ProfileState>,
    pub selected: usize,
    pub focus: Focus,
    pub overlay: Overlay,
    pub input: redstone_core::editor::InputState,
    pub console_buffers: HashMap<String, ConsoleBuffer>,
    pub should_quit: bool,
    pub rects: LayoutRects,
    pub mouse_capture: bool,
    daemon_clients: HashMap<String, redstone_core::ipc::DaemonClient>,
    pending_starts: HashMap<String, Instant>,
    tx: mpsc::Sender<Event>,
}

pub fn spawn_daemon_task(name: String, tx: mpsc::Sender<Event>) {
    tokio::spawn(async move {
        let mut client = None;
        for i in 0..6 {
            if i > 0 {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
            let Ok(c) = redstone_core::ipc::DaemonClient::connect(&name).await else {
                continue;
            };
            client = Some(c);
            break;
        }
        let Some(mut client) = client else {
            let _ = tx.try_send(Event::DaemonMessage {
                profile: name,
                line: "[ERROR] Failed to connect to daemon (gave up after 6 attempts)\n"
                    .to_string(),
            });
            return;
        };
        if client.subscribe().await.is_err() {
            return;
        }
        let _ = tx
            .send(Event::DaemonConnected {
                profile: name.clone(),
            })
            .await;
        loop {
            let resp = match client.read_response().await {
                Ok(r) => r,
                Err(_) => break,
            };
            match resp {
                redstone_core::ipc::DaemonResponse::Stdout { data }
                | redstone_core::ipc::DaemonResponse::Stderr { data } => {
                    if tx
                        .send(Event::DaemonMessage {
                            profile: name.clone(),
                            line: data,
                        })
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                redstone_core::ipc::DaemonResponse::Exited { .. } => break,
                _ => {}
            }
        }
    });
}

impl State {
    pub fn new(tx: mpsc::Sender<Event>) -> Self {
        Self {
            profiles: Vec::new(),
            selected: 0,
            focus: Focus::ServerList,
            overlay: Overlay::None,
            input: redstone_core::editor::InputState::new(),
            console_buffers: HashMap::new(),
            should_quit: false,
            rects: LayoutRects {
                server_list: Rect::default(),
                console: Rect::default(),
                status_panel: Rect::default(),
            },
            daemon_clients: HashMap::new(),
            pending_starts: HashMap::new(),
            mouse_capture: true,
            tx,
        }
    }

    fn push_console_error(&mut self, profile: &str, msg: &str) {
        if let Some(buf) = self.console_buffers.get_mut(profile) {
            buf.push(format!("[ERROR] {}\n", msg));
        }
    }

    pub fn refresh_profiles(&mut self) {
        let selected_name = self.profiles.get(self.selected).map(|p| p.name.clone());
        let entries = redstone_core::profile::list_all_profiles();
        let disk_names: HashSet<String> = entries.into_iter().map(|e| e.name).collect();

        let existing_names: Vec<String> = self.profiles.iter().map(|p| p.name.clone()).collect();

        // Expire pending starts older than 10 seconds
        let now = Instant::now();
        self.pending_starts
            .retain(|_, time| now.duration_since(*time).as_secs() < 10);

        for name in &disk_names {
            let server_state = redstone_core::profile::read_server_state(name)
                .ok()
                .flatten();
            let status = match server_state {
                Some(s) if s.running => ConnectionStatus::Running,
                _ => ConnectionStatus::Offline,
            };

            if let Some(pos) = existing_names.iter().position(|n| n == name) {
                // Only upgrade (Offline → Running) from disk; downgrade only if not pending
                if status == ConnectionStatus::Running {
                    self.profiles[pos].status = ConnectionStatus::Running;
                } else if !self.pending_starts.contains_key(name.as_str()) {
                    self.profiles[pos].status = ConnectionStatus::Offline;
                }
            } else {
                self.profiles.push(ProfileState {
                    name: name.clone(),
                    status,
                });
                self.console_buffers
                    .entry(name.clone())
                    .or_insert_with(ConsoleBuffer::new);
            }
        }

        self.profiles
            .retain(|p| disk_names.contains(p.name.as_str()));

        self.profiles.sort_by(|a, b| {
            let a_running = a.status == ConnectionStatus::Running;
            let b_running = b.status == ConnectionStatus::Running;
            b_running.cmp(&a_running).then(a.name.cmp(&b.name))
        });

        // Restore selection by name (not index) to survive sort reorder
        if let Some(ref name) = selected_name {
            if let Some(pos) = self.profiles.iter().position(|p| &p.name == name) {
                self.selected = pos;
            } else if self.profiles.is_empty() {
                self.selected = 0;
            } else if self.selected >= self.profiles.len() {
                self.selected = self.profiles.len() - 1;
            } else {
                self.selected = 0;
            }
        } else if self.profiles.is_empty() {
            self.selected = 0;
        } else if self.selected >= self.profiles.len() {
            self.selected = self.profiles.len() - 1;
        }

    }

    pub fn selected_profile(&self) -> Option<&ProfileState> {
        self.profiles.get(self.selected)
    }

    pub fn daemon_connected(&mut self, name: &str) {
        self.pending_starts.remove(name);
    }


    pub fn spawn_daemon_tasks(&mut self) {
        for i in 0..self.profiles.len() {
            if self.profiles[i].status != ConnectionStatus::Running {
                continue;
            }
            let name = self.profiles[i].name.clone();
            let tx = self.tx.clone();
            spawn_daemon_task(name, tx);
        }
    }

    pub async fn send_command(&mut self, profile: &str, line: &str) {
        if let Some(client) = self.daemon_clients.get_mut(profile) {
            if let Err(e) = client.write_stdin(line).await {
                self.push_console_error(profile, &format!("Failed to send command: {}", e));
                self.daemon_clients.remove(profile);
            }
            return;
        }
        let mut client = 'connect: {
            for attempt in 0..4 {
                if attempt > 0 {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
                match redstone_core::ipc::DaemonClient::connect(profile).await {
                    Ok(c) => break 'connect c,
                    Err(e) => {
                        if attempt == 0 {
                            self.push_console_error(profile, &format!("Failed to connect: {}", e));
                        }
                    }
                }
            }
            self.push_console_error(profile, "Gave up connecting to daemon");
            return;
        };
        if let Err(e) = client.write_stdin(line).await {
            self.push_console_error(profile, &format!("Failed to send command: {}", e));
        }
        if let Err(e) = client.subscribe().await {
            self.push_console_error(profile, &format!("Failed to subscribe: {}", e));
        }
        self.daemon_clients.insert(profile.to_string(), client);
    }

    pub async fn start_server(&mut self, name: &str) {
        self.daemon_clients.remove(name);
        let exe = match std::env::current_exe() {
            Ok(p) => p,
            Err(e) => {
                self.push_console_error(name, &format!("Failed to get executable path: {}", e));
                return;
            }
        };

        let yaml_path = redstone_core::profile::profile_yaml_path(name);
        if !yaml_path.exists() {
            self.push_console_error(
                name,
                &format!("Profile YAML not found: {}", yaml_path.display()),
            );
            return;
        }

        let msg = format!("[TUI] Starting '{}'...\n", name);
        if let Some(buf) = self.console_buffers.get_mut(name) {
            buf.lines.push_back(msg);
        }

        let yaml_str = yaml_path.to_string_lossy().into_owned();

        if redstone_core::daemon::check_daemon_alive(name).await {
            self.push_console_error(name, &format!("Daemon '{}' is already running", name));
            return;
        }

        let (mut child, stderr) = match redstone_core::daemon::spawn_daemon(&exe, &yaml_str) {
            Ok(pair) => pair,
            Err(e) => {
                self.push_console_error(name, &format!("Failed to start daemon: {}", e));
                return;
            }
        };

        // Capture daemon stderr in background (runtime errors)
        let tx = self.tx.clone();
        let name_stderr = name.to_string();
        tokio::task::spawn_blocking(move || {
            use std::io::BufRead;
            let reader = std::io::BufReader::new(stderr);
            for line in reader.lines() {
                match line {
                    Ok(l) => {
                        let msg = l.trim().to_string();
                        if !msg.is_empty() {
                            let _ = tx.blocking_send(Event::DaemonMessage {
                                profile: name_stderr.clone(),
                                line: format!("[daemon] {}\n", msg),
                            });
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        // Liveness check: daemon exited immediately?
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        if let Ok(Some(status)) = child.try_wait() {
            self.push_console_error(
                name,
                &format!("Daemon exited immediately (status: {})", status),
            );
            return;
        }

        if let Some(pos) = self.profiles.iter().position(|p| p.name == name) {
            self.profiles[pos].status = ConnectionStatus::Running;
        }
        self.pending_starts.insert(name.to_string(), Instant::now());
        let tx = self.tx.clone();
        spawn_daemon_task(name.to_string(), tx);
    }

    pub async fn stop_server(&mut self, name: &str) {
        if let Some(pos) = self.profiles.iter().position(|p| p.name == name) {
            self.profiles[pos].status = ConnectionStatus::Offline;
        }

        let mut client = match redstone_core::ipc::DaemonClient::connect(name).await {
            Ok(c) => c,
            Err(e) => {
                self.push_console_error(name, &format!("Failed to connect for stop: {}", e));
                return;
            }
        };
        if let Err(e) = client.write_stdin("stop\n").await {
            self.push_console_error(name, &format!("Failed to send stop: {}", e));
        }
    }

    pub async fn kill_server(&mut self, name: &str) {
        match redstone_core::daemon::kill_server(name).await {
            Ok(_) => {
                if let Some(pos) = self.profiles.iter().position(|p| p.name == name) {
                    self.profiles[pos].status = ConnectionStatus::Offline;
                }
            }
            Err(e) => {
                self.push_console_error(name, &e);
            }
        }
    }

    pub async fn restart_server(&mut self, name: &str) {
        self.daemon_clients.remove(name);

        let msg = format!("[TUI] Restarting '{}'...\n", name);
        if let Some(buf) = self.console_buffers.get_mut(name) {
            buf.lines.push_back(msg);
        }

        let tx = self.tx.clone();
        let name = name.to_string();

        tokio::spawn(async move {
            {
                let mut client = match redstone_core::ipc::DaemonClient::connect(&name).await {
                    Ok(c) => c,
                    Err(_) => return,
                };
                let _ = client.write_stdin("stop\n").await;
            }

            // Wait 5s for daemon to begin shutdown
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;

            // Poll every 1s for socket to disappear (up to 55s more)
            let mut gone = false;
            for _ in 0..55 {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                if redstone_core::ipc::DaemonClient::connect(&name)
                    .await
                    .is_err()
                {
                    gone = true;
                    break;
                }
            }

            if !gone {
                return;
            }

            // Double-check: wait 1s and confirm socket is still gone
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            if redstone_core::ipc::DaemonClient::connect(&name)
                .await
                .is_ok()
            {
                return;
            }

            let _ = tx.send(Event::StartServer { profile: name }).await;
        });
    }
}
