// redstone-tui/src/ui.rs
use crate::state::{ConnectionStatus, Focus, Overlay, State};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use rust_i18n::t;

const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn draw(f: &mut Frame, state: &mut State) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(10)])
        .split(f.area());

    draw_header(f, chunks[0], state);

    let main = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(28), Constraint::Min(40)])
        .split(chunks[1]);

    let left = main[0];
    let right = main[1];

    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(12), Constraint::Min(10)])
        .split(right);

    state.rects.server_list = left;
    state.rects.console = right_chunks[1];
    state.rects.status_panel = right_chunks[0];

    draw_server_list(f, left, state);
    draw_status_panel(f, right_chunks[0], state);

    let console_buf = state
        .selected_profile()
        .and_then(|p| state.console_buffers.get(&p.name));
    if let Some(buf) = console_buf {
        crate::console::draw_console(f, right_chunks[1], buf, &state.input, state.focus);
    }

    if state.overlay != Overlay::None {
        draw_overlay(f, state);
    }
}

fn draw_overlay(f: &mut Frame, state: &State) {
    let area = f.area();
    let overlay_area = Rect {
        x: area.width.saturating_sub(50) / 2,
        y: area.height.saturating_sub(5) / 2,
        width: 50.min(area.width),
        height: 5.min(area.height),
    };

    let text = match &state.overlay {
        Overlay::ConfirmKill(name) => {
            format!("\n{}\n", t!("app.cli.kill.confirm", name = name))
        }
        Overlay::None => unreachable!(),
    };

    f.render_widget(Clear, overlay_area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(t!("app.tui.overlay.title"))
        .style(Style::default().bg(Color::Black));
    let para = Paragraph::new(text)
        .block(block)
        .alignment(Alignment::Center);
    f.render_widget(para, overlay_area);
}

fn draw_header(f: &mut Frame, area: Rect, state: &State) {
    let online = state
        .profiles
        .iter()
        .filter(|p| p.status == ConnectionStatus::Running)
        .count();
    let total = state.profiles.len();

    let text = Line::from(vec![
        Span::styled(
            format!(" Redstone v{} ", VERSION),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("|"),
        Span::styled(
            format!(
                " {} ",
                t!(
                    "app.tui.header.online_total",
                    online = online,
                    total = total
                )
            ),
            Style::default().fg(Color::White),
        ),
        Span::raw("|"),
        Span::styled(
            format!(" {} ", t!("app.tui.header.help")),
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    f.render_widget(
        Paragraph::new(text).style(Style::default().bg(Color::Black)),
        area,
    );
}

fn draw_server_list(f: &mut Frame, area: Rect, state: &State) {
    let title = match state.focus {
        Focus::ServerList => t!("app.tui.server_list.title_focused"),
        Focus::Console => t!("app.tui.server_list.title"),
    };

    let border_style = match state.focus {
        Focus::ServerList => Style::default().fg(Color::Cyan),
        Focus::Console => Style::default().fg(Color::White),
    };

    if state.profiles.is_empty() {
        let para = Paragraph::new(t!("app.tui.server_list.empty"))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .border_style(border_style),
            )
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(para, area);
        return;
    }

    let items: Vec<ListItem> = state
        .profiles
        .iter()
        .map(|p| {
            let (icon, style) = match p.status {
                ConnectionStatus::Running => ("🟢", Style::default().fg(Color::Green)),
                ConnectionStatus::Offline => ("⚪", Style::default().fg(Color::Gray)),
            };
            ListItem::new(Line::from(vec![
                Span::raw(format!("{} ", icon)),
                Span::styled(&p.name, style),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .border_style(border_style),
        )
        .highlight_style(
            Style::default()
                .add_modifier(Modifier::REVERSED)
                .bg(Color::DarkGray),
        )
        .highlight_symbol("> ");

    let mut list_state = ratatui::widgets::ListState::default().with_selected(Some(state.selected));
    f.render_stateful_widget(list, area, &mut list_state);
}

fn format_uptime(started_at: Option<u64>) -> String {
    started_at
        .map(|t| {
            let elapsed = redstone_core::profile::now_epoch().saturating_sub(t);
            if elapsed < 60 {
                format!("{}s", elapsed)
            } else if elapsed < 3600 {
                format!("{}m {}s", elapsed / 60, elapsed % 60)
            } else {
                format!(
                    "{}h {}m {}s",
                    elapsed / 3600,
                    (elapsed % 3600) / 60,
                    elapsed % 60
                )
            }
        })
        .unwrap_or_else(|| "?".into())
}

fn draw_status_panel(f: &mut Frame, area: Rect, state: &State) {
    let text = if let Some(p) = state.selected_profile() {
        let server_state = redstone_core::profile::read_server_state(&p.name)
            .ok()
            .flatten();
        let info = match server_state {
            Some(s) if s.running => {
                let pid = s.pid.map(|p| p.to_string()).unwrap_or_else(|| "?".into());
                let uptime = format_uptime(s.started_at);
                t!(
                    "app.tui.status.running",
                    name = p.name,
                    pid = pid,
                    uptime = uptime
                )
            }
            _ => t!("app.tui.status.offline", name = p.name),
        };
        let buttons: Vec<Span> = if p.status == ConnectionStatus::Running {
            vec![
                Span::styled(
                    t!("app.tui.status.btn_stop"),
                    Style::default().fg(Color::Yellow),
                ),
                Span::raw("  "),
                Span::styled(
                    t!("app.tui.status.btn_kill"),
                    Style::default().fg(Color::Red),
                ),
                Span::raw("  "),
                Span::styled(
                    t!("app.tui.status.btn_restart"),
                    Style::default().fg(Color::Cyan),
                ),
            ]
        } else {
            vec![Span::styled(
                t!("app.tui.status.btn_start"),
                Style::default().fg(Color::Green),
            )]
        };
        let buttons_line = Line::from(buttons);

        let slp_extra = state
            .slp_cache
            .get(&p.name)
            .map(|slp| {
                format!(
                    "\n Players: {}/{}\n Latency: {}ms\n Version: {}",
                    slp.online_players, slp.max_players, slp.latency_ms, slp.version
                )
            })
            .unwrap_or_default();

        format!("{}\n{}{}", info, buttons_line, slp_extra)
    } else {
        t!("app.tui.status.no_selection").to_string()
    };

    let p = Paragraph::new(text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(t!("app.tui.status.title")),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}
