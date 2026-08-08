//! Rendering for the keymap modal popup (Ctrl+H).

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::keymap_menu::state::KeymapMenu;
use crate::theme;

/// Render the keymap popup as a centered modal.
pub fn render_keymap_popup(f: &mut Frame, area: Rect, menu: &KeymapMenu) {
    let rows = menu.len() as u16;
    let want_h = 3 + rows + 1; // border-top + rows + footer
    let h = want_h.min(area.height.saturating_sub(2));
    let w = 72u16.min(area.width.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let popup = Rect::new(x, y, w, h);
    f.render_widget(Clear, popup);

    let block = theme::rounded_block("Keyboard Shortcuts");

    let sel_st = Style::default()
        .fg(theme::accent())
        .add_modifier(Modifier::BOLD);
    let val_st = Style::default().fg(theme::text());
    let dim_st = Style::default().fg(theme::muted());
    let cap_st = Style::default()
        .fg(theme::warn_color())
        .add_modifier(Modifier::BOLD);

    let mut lines: Vec<Line> = Vec::new();
    for (i, (key, label, spec)) in menu.entries().iter().enumerate() {
        let is_sel = i == menu.selected;
        let marker = if is_sel { "❯ " } else { "  " };
        let st = if is_sel { sel_st } else { val_st };

        let mut spans = vec![
            Span::styled(marker, st),
            Span::styled(format!("{:<7} ", spec), st),
            Span::styled(label.clone(), if is_sel { st } else { dim_st }),
        ];

        if is_sel && menu.capturing {
            spans[1] = Span::styled("Press a key...  ", cap_st);
        }
        let _ = key; // key is the config key, not shown in the row
        lines.push(Line::from(spans));
    }

    lines.push(Line::from(Span::styled(
        " Enter: rebind   Ctrl+R: reset to default   Esc: close   Ctrl+D: quit",
        dim_st,
    )));

    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .alignment(Alignment::Left)
            .wrap(Wrap { trim: false }),
        popup,
    );
}
