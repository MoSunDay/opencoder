//! Rendering for the `/cache_salt` read-only panel. State in [`super::state`].

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use super::state::CacheSaltMenu;

/// Draw the salt panel as a centered modal: parent row first (highlighted with
/// `*`), then every subagent of the current session with its lifecycle status.
pub fn render_cache_salt_popup(f: &mut Frame, area: Rect, menu: &CacheSaltMenu) {
    let rows = menu.entries.len() as u16;
    let want_h = 3 + rows + if menu.enabled { 0 } else { 1 };
    let h = want_h.min(area.height.saturating_sub(2));
    let w = 76u16.min(area.width.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let popup = Rect::new(x, y, w, h);
    f.render_widget(Clear, popup);

    let state_word = if menu.enabled { "enabled" } else { "DISABLED" };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Prefix-cache salt (cache_salt: {state_word}) "));

    let parent_st = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let val = Style::default().fg(Color::White);
    let dim = Style::default().fg(Color::DarkGray);

    let mut lines: Vec<Line> = Vec::new();
    for e in &menu.entries {
        let st = if e.is_parent { parent_st } else { val };
        let mark = if e.is_parent { "*" } else { " " };
        let mut spans = vec![
            Span::styled(format!("{mark} {:<7} ", e.role), st),
            Span::styled(e.salt.clone(), st),
        ];
        if !e.status.is_empty() {
            spans.push(Span::styled(format!("   [{}]", e.status), dim));
        }
        lines.push(Line::from(spans));
    }
    if !menu.enabled {
        lines.push(Line::from(Span::styled(
            " cache_salt disabled \u{2014} these salts are NOT sent on requests.",
            Style::default().fg(Color::Yellow),
        )));
    }
    lines.push(Line::from(Span::styled(
        " * current session  |  Esc / Enter to close",
        dim,
    )));

    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .alignment(Alignment::Left)
            .wrap(Wrap { trim: false }),
        popup,
    );
}
