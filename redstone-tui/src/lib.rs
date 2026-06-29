// redstone-tui/src/lib.rs
mod console;
mod event;
mod state;
mod ui;

rust_i18n::i18n!("../redstone-i18n/locales", fallback = "en");

use crate::event::{Event, EventLoop};
use crate::state::{ConnectionStatus, Focus, Overlay, State};
use crate::ui::draw;
use crossterm::ExecutableCommand;
use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, KeyCode, KeyEventKind, KeyModifiers, MouseButton,
    MouseEventKind,
};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io;

pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = redstone_core::config::RedstoneConfig::load().unwrap_or(
        redstone_core::config::RedstoneConfig {
            locale: None,
            tui_fps: 30,
        },
    );

    let fps = cfg.tui_fps.clamp(1, 120);
    let tick_rate = std::time::Duration::from_secs_f64(1.0 / fps as f64);

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    stdout.execute(EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let event_loop = EventLoop::new(tick_rate);
    let tx = event_loop.sender();
    let mut state = State::new(tx);

    state.refresh_profiles();
    state.spawn_daemon_tasks();

    let res = run_app(&mut terminal, &mut state, event_loop).await;

    terminal.backend_mut().execute(DisableMouseCapture)?;
    disable_raw_mode()?;
    terminal.backend_mut().execute(LeaveAlternateScreen)?;

    if let Err(e) = res {
        eprintln!("TUI error: {}", e);
    }
    Ok(())
}

fn get_button_click(state: &State, col: u16, row: u16) -> Option<&'static str> {
    let p = state.selected_profile()?;
    let rect = state.rects.status_panel;

    if !rect.intersects(ratatui::layout::Rect::new(col, row, 1, 1)) {
        return None;
    }

    // Button row is inside the border; content is 4 lines for Offline or 5 for Running
    let btn_row_start = rect.y + 4;
    let btn_row_end = rect.y + 5;
    if row < btn_row_start || row > btn_row_end {
        return None;
    }

    let inner_col = col.saturating_sub(rect.x).saturating_sub(1) as i16;

    if p.status == ConnectionStatus::Running {
        if (0..=8).contains(&inner_col) {
            Some("stop")
        } else if (10..=19).contains(&inner_col) {
            Some("kill")
        } else if (21..=31).contains(&inner_col) {
            Some("restart")
        } else {
            None
        }
    } else if p.status == ConnectionStatus::Offline {
        if (0..=9).contains(&inner_col) {
            Some("start")
        } else {
            None
        }
    } else {
        None
    }
}

async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut State,
    mut event_loop: EventLoop,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut last_refresh = std::time::Instant::now();
    let refresh_interval = std::time::Duration::from_secs(5);

    loop {
        terminal.draw(|f| draw(f, state))?;

        let Some(event) = event_loop.recv().await else {
            break;
        };

        match event {
            Event::Key(key) => {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                // Tab always cycles focus (never inserted into input buffer)
                if key.code == KeyCode::Tab && key.modifiers == KeyModifiers::NONE {
                    state.focus = match state.focus {
                        Focus::ServerList => Focus::Console,
                        Focus::Console => Focus::ServerList,
                    };
                    continue;
                }

                // Esc: return to ServerList, or quit if already there
                if key.code == KeyCode::Esc && key.modifiers == KeyModifiers::NONE {
                    match state.focus {
                        Focus::Console => state.focus = Focus::ServerList,
                        Focus::ServerList => state.should_quit = true,
                    }
                    continue;
                }

                // F1 toggles mouse capture (for text selection)
                if key.code == KeyCode::F(1) && key.modifiers == KeyModifiers::NONE {
                    state.mouse_capture = !state.mouse_capture;
                    if state.mouse_capture {
                        let _ = io::stdout().execute(EnableMouseCapture);
                    } else {
                        let _ = io::stdout().execute(DisableMouseCapture);
                    }
                    continue;
                }

                // Handle overlay first (e.g. ConfirmKill)
                let overlay = state.overlay.clone();
                match overlay {
                    Overlay::ConfirmKill(name) => {
                        state.overlay = Overlay::None;
                        if matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y')) {
                            state.kill_server(&name).await;
                        }
                        continue;
                    }
                    Overlay::None => {}
                }

                match state.focus {
                    Focus::ServerList => {
                        if key.modifiers == KeyModifiers::CONTROL {
                            if matches!(key.code, KeyCode::Char('q') | KeyCode::Char('c')) {
                                state.should_quit = true;
                            }
                        } else {
                            match key.code {
                                KeyCode::Up => {
                                    if state.selected > 0 {
                                        state.selected -= 1;
                                    }
                                }
                                KeyCode::Down => {
                                    if state.selected + 1 < state.profiles.len() {
                                        state.selected += 1;
                                    }
                                }
                                KeyCode::PageUp => {
                                    state.selected = state.selected.saturating_sub(10);
                                }
                                KeyCode::PageDown => {
                                    state.selected = (state.selected + 10)
                                        .min(state.profiles.len().saturating_sub(1));
                                }
                                KeyCode::Home => {
                                    state.selected = 0;
                                }
                                KeyCode::End => {
                                    state.selected = state.profiles.len().saturating_sub(1);
                                }
                                KeyCode::Char('s') | KeyCode::Char('S') => {
                                    let name = state
                                        .selected_profile()
                                        .filter(|p| p.status == ConnectionStatus::Offline)
                                        .map(|p| p.name.clone());
                                    if let Some(name) = name {
                                        state.start_server(&name).await;
                                    }
                                }
                                KeyCode::Char('t') | KeyCode::Char('T') => {
                                    let name = state
                                        .selected_profile()
                                        .filter(|p| p.status == ConnectionStatus::Running)
                                        .map(|p| p.name.clone());
                                    if let Some(name) = name {
                                        state.stop_server(&name).await;
                                    }
                                }
                                KeyCode::Char('k') | KeyCode::Char('K') => {
                                    let name = state
                                        .selected_profile()
                                        .filter(|p| p.status == ConnectionStatus::Running)
                                        .map(|p| p.name.clone());
                                    if let Some(name) = name {
                                        state.overlay = Overlay::ConfirmKill(name);
                                    }
                                }
                                KeyCode::Char('r') | KeyCode::Char('R') => {
                                    let name = state
                                        .selected_profile()
                                        .filter(|p| p.status == ConnectionStatus::Running)
                                        .map(|p| p.name.clone());
                                    if let Some(name) = name {
                                        state.restart_server(&name).await;
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    Focus::Console => {
                        let action = state.input.handle_key(key);
                        match action {
                            redstone_core::editor::InputAction::Quit => {
                                state.should_quit = true;
                            }
                            redstone_core::editor::InputAction::Submit(line) => {
                                let name = state.selected_profile().map(|p| p.name.clone());
                                if let Some(ref name) = name {
                                    let mut to_send = line;
                                    to_send.push('\n');
                                    state.send_command(name, &to_send).await;
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            Event::Mouse(mouse) => {
                if !state.mouse_capture {
                    continue;
                }
                if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                    let col = mouse.column;
                    let row = mouse.row;
                    let pos_rect = ratatui::layout::Rect::new(col, row, 1, 1);

                    if state.rects.server_list.intersects(pos_rect) {
                        state.focus = Focus::ServerList;
                        let rel_row = row
                            .saturating_sub(state.rects.server_list.y)
                            .saturating_sub(1);
                        if rel_row < state.profiles.len() as u16 {
                            state.selected = rel_row as usize;
                        }
                    } else if state.rects.console.intersects(pos_rect) {
                        state.focus = Focus::Console;
                    }

                    if let Some(action) = get_button_click(state, col, row) {
                        let name = state.selected_profile().map(|p| p.name.clone());
                        if let Some(ref name) = name {
                            match action {
                                "start" => state.start_server(name).await,
                                "stop" => state.stop_server(name).await,
                                "kill" => state.overlay = Overlay::ConfirmKill(name.clone()),
                                "restart" => state.restart_server(name).await,
                                _ => {}
                            }
                        }
                    }
                }
            }
            Event::DaemonMessage { profile, line } => {
                if let Some(buf) = state.console_buffers.get_mut(&profile) {
                    buf.push(line);
                }
            }
            Event::DaemonConnected { profile } => {
                state.daemon_connected(&profile);
            }
            Event::StartServer { profile } => {
                state.start_server(&profile).await;
            }
            Event::Resize(w, h) => {
                let _ = (w, h);
                terminal.draw(|f| draw(f, state))?;
            }
            Event::Tick if last_refresh.elapsed() >= refresh_interval => {
                state.refresh_profiles();
                last_refresh = std::time::Instant::now();
            }
            _ => {}
        }

        if state.should_quit {
            break;
        }
    }

    Ok(())
}
