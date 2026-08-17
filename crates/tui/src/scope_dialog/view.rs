//! Rendering for the `inject_to` multi-select dialog: a bordered checkbox
//! list centered over the underlying form area.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

use super::{ScopeDialog, OPTIONS};

fn focus() -> Style {
    Style::default()
        .fg(crate::theme::warn_color())
        .add_modifier(Modifier::BOLD)
}

fn dim() -> Style {
    Style::default().fg(crate::theme::subtle())
}

/// Centered overlay box for the dialog. Height grows by one line when the
/// "at least one target" reminder is showing.
pub fn dialog_area(area: Rect, any_checked: bool) -> Rect {
    let h = if any_checked { 6 } else { 7 };
    let w = 48u16.min(area.width.saturating_sub(2));
    let h = h.min(area.height.saturating_sub(1).max(1));
    Rect::new(
        area.x + area.width.saturating_sub(w) / 2,
        area.y + area.height.saturating_sub(h) / 2,
        w,
        h,
    )
}

pub fn render_scope_dialog(frame: &mut Frame, area: Rect, dialog: &ScopeDialog) {
    let popup = dialog_area(area, dialog.any_checked());
    frame.render_widget(Clear, popup);
    let mut lines = Vec::new();
    for (idx, opt) in OPTIONS.iter().enumerate() {
        let selected = dialog.cursor() == idx;
        let base = if selected {
            focus()
        } else {
            Style::default().fg(crate::theme::text())
        };
        let check = if dialog.checked(idx) { "[x]" } else { "[ ]" };
        lines.push(Line::from(vec![
            Span::styled(" ", dim()),
            Span::styled(format!("{check} "), base),
            Span::styled(format!("{opt:<10}"), base),
            if selected {
                Span::styled("←", dim())
            } else {
                Span::raw("")
            },
        ]));
    }
    if !dialog.any_checked() {
        lines.push(Line::styled(
            " (at least one target required)",
            Style::default().fg(crate::theme::err_color()),
        ));
    }
    let title = " inject to — ↑/↓ move, Space toggle, Enter confirm, Esc cancel ";
    frame.render_widget(
        Paragraph::new(lines).block(crate::theme::rounded_block_plain().title(title)),
        popup,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencoder_core::InjectionTarget;
    use ratatui::{backend::TestBackend, Terminal as TestTerminal};

    /// The overlay draws one `[x]`/`[ ]` row per option, centered over the
    /// form area, and clamps to tiny terminals without panicking.
    #[test]
    fn renders_checkbox_rows_and_survives_tiny_area() {
        let mut dialog = ScopeDialog::new(InjectionTarget::subagents());
        // cursor on row 1 (explore), already checked
        dialog.handle_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Down,
            crossterm::event::KeyModifiers::NONE,
        ));
        for (width, height) in [(60u16, 20u16), (20, 8)] {
            let mut terminal = TestTerminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|f| render_scope_dialog(f, f.area(), &dialog))
                .expect("render must not panic");
            let text = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|c| c.symbol())
                .collect::<String>();
            assert!(
                text.contains("[x] explore"),
                "checked row rendered at {width}x{height}"
            );
            assert!(
                text.contains("[ ] parent"),
                "unchecked row rendered at {width}x{height}"
            );
        }
        // Degenerate size: must clamp and render without panicking.
        let mut tiny = TestTerminal::new(TestBackend::new(10, 4)).unwrap();
        tiny.draw(|f| render_scope_dialog(f, f.area(), &dialog))
            .expect("tiny render must not panic");
    }
}
