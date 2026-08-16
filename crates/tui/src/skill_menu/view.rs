//! Rendering for the `/skill` modal (mirrors the `/mcp` list view).

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

use super::list::SkillList;
use super::state::SkillMenu;

/// Width reserved for the name column (truncated + padded).
const NAME_W: usize = 18;

fn focus_style() -> Style {
    Style::default()
        .fg(crate::theme::warn_color())
        .add_modifier(Modifier::BOLD)
}

fn dim_style() -> Style {
    Style::default().fg(crate::theme::subtle())
}

fn truncate(s: &str, w: usize) -> String {
    s.chars().take(w).collect()
}

/// Draw the `/skill` toggle list as a centered popup anchored just above the
/// composer (same geometry rules as `render_mcp_popup`'s list view).
pub fn render_skill_popup(f: &mut Frame, area: Rect, composer_top: u16, menu: &SkillMenu) {
    let SkillMenu::List(list) = menu;
    render_list(f, area, composer_top, list);
}

fn render_list(f: &mut Frame, area: Rect, composer_top: u16, list: &SkillList) {
    let n = list.entries.len() as u16;
    // borders + rows + 1 footer hint row; capped like the /mcp popup.
    let want_h = n.max(3) + 4;
    let h = want_h.min(20u16).min(composer_top.max(1));
    let w = 76u16.min(area.width.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = composer_top.saturating_sub(h);
    let popup = Rect::new(x, y, w, h);
    f.render_widget(Clear, popup);

    let block = crate::theme::rounded_block_plain().title(" /skill — 默认注入的 skills ");

    let mut lines: Vec<Line> = Vec::new();
    if list.entries.is_empty() {
        lines.push(Line::styled(
            " ~/.opencoder/skills 下未发现 skill",
            Style::default().fg(crate::theme::warn_color()),
        ));
    } else {
        // inner width; description gets whatever the badge + name leave.
        let inner = w.saturating_sub(2) as usize;
        let desc_w = inner.saturating_sub(NAME_W + 8);
        for (i, entry) in list.entries.iter().enumerate() {
            let selected = i == list.selected;
            let line_style = if selected {
                focus_style()
            } else if entry.enabled {
                Style::default().fg(crate::theme::accent())
            } else {
                dim_style()
            };
            let switch = if entry.enabled { "[ON]" } else { "[OFF]" };
            let switch_style = if entry.enabled {
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD)
            } else {
                dim_style()
            };
            let name = truncate(&entry.name, NAME_W);
            let desc = truncate(&entry.description, desc_w);
            let spans = vec![
                Span::styled(format!("{switch:<5} "), switch_style),
                Span::styled(format!("{name:<NAME_W$} "), line_style),
                Span::styled(desc, line_style),
            ];
            lines.push(Line::from(spans));
        }
    }
    lines.push(Line::styled(
        " ↑/↓ 选择 · ←/→ 切换 · Enter/Esc 关闭",
        Style::default().fg(crate::theme::muted()),
    ));

    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .alignment(Alignment::Left),
        popup,
    );
}
