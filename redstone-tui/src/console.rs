// redstone-tui/src/console.rs
use crate::state::{ConsoleBuffer, Focus};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use rust_i18n::t;
use unicode_width::UnicodeWidthStr;

pub fn draw_console(
    f: &mut Frame,
    area: Rect,
    buf: &ConsoleBuffer,
    input: &redstone_core::editor::InputState,
    focus: Focus,
) {
    let title = match focus {
        Focus::Console => t!("app.tui.console.title_focused"),
        Focus::ServerList => t!("app.tui.console.title"),
    };

    let border_style = match focus {
        Focus::Console => Style::default().fg(Color::Cyan),
        Focus::ServerList => Style::default().fg(Color::White),
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(border_style);
    f.render_widget(&block, area);

    let inner = block.inner(area);

    let input_area = Rect {
        y: inner.y,
        height: 3,
        width: inner.width,
        x: inner.x,
    };
    let log_area = Rect {
        y: inner.y + 3,
        height: inner.height.saturating_sub(3),
        width: inner.width,
        x: inner.x,
    };

    let input_prefix = t!("app.tui.console.input_prompt");
    let input_prefix_width = input_prefix.width();
    let cursor_visual = input.input[..input.cursor].width();
    let input_display = format!("{}{}", input_prefix, input.input);

    let cursor_col = (inner.x + input_prefix_width as u16 + cursor_visual as u16) as u16;
    let cursor_row = input_area.y;

    let input_style = Style::default().fg(if focus == Focus::Console {
        Color::Green
    } else {
        Color::Gray
    });
    let input_para = Paragraph::new(Line::from(Span::raw(input_display))).style(input_style);
    f.render_widget(input_para, input_area);

    if focus == Focus::Console {
        f.set_cursor_position((cursor_col, cursor_row));
    }

    let log_height = log_area.height as usize;
    let log_lines: Vec<Line> = buf
        .visible_lines(log_height)
        .into_iter()
        .map(|l| Line::from(Span::raw(l)))
        .collect();

    let scroll_indicator = if !buf.is_at_bottom() {
        format!(" ({})", buf.scroll_offset)
    } else {
        String::new()
    };
    let log_title = format!(
        "{} {}{}",
        t!("app.tui.console.log_title"),
        log_lines.len(),
        scroll_indicator
    );

    let log_para = Paragraph::new(log_lines)
        .style(Style::default().fg(Color::White))
        .block(
            Block::default()
                .title(log_title)
                .title_alignment(ratatui::layout::Alignment::Right),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(log_para, log_area);
}
