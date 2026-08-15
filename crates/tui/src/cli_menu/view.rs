use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

use super::content_dialog::{ContentDialog, VIEW_LINES};
use super::{CliField, CliForm, CliList, CliMenu};

fn focus() -> Style {
    Style::default()
        .fg(crate::theme::warn_color())
        .add_modifier(Modifier::BOLD)
}

fn dim() -> Style {
    Style::default().fg(crate::theme::subtle())
}

pub fn render_cli_popup(frame: &mut Frame, area: Rect, composer_top: u16, menu: &CliMenu) {
    match menu {
        CliMenu::List(list) => render_list(frame, area, composer_top, list),
        CliMenu::Form(form) => {
            render_form(frame, area, composer_top, form);
            if let Some(dialog) = form.scope_dialog.as_ref() {
                crate::scope_dialog::render_scope_dialog(frame, area, dialog);
            }
            if let Some(dialog) = form.content_dialog.as_ref() {
                render_content_dialog(frame, area, dialog);
            }
        }
    }
}

fn popup(area: Rect, composer_top: u16, height: u16) -> Rect {
    let h = height.min(composer_top.max(1));
    let w = 88u16.min(area.width.saturating_sub(4));
    Rect::new(
        area.x + area.width.saturating_sub(w) / 2,
        composer_top.saturating_sub(h),
        w,
        h,
    )
}

fn render_list(frame: &mut Frame, area: Rect, top: u16, list: &CliList) {
    let area = popup(area, top, (list.entries.len() as u16).max(4) + 5);
    frame.render_widget(Clear, area);
    let title = if list.confirm_delete.is_some() {
        " /cli — CONFIRM DELETE? y=delete, n/Esc=cancel "
    } else {
        " /cli — ↑/↓ select, ←/→ toggle, e=edit, n=new, d=delete, Enter/Esc close "
    };
    let mut lines = Vec::new();
    if list.entries.is_empty() {
        lines.push(Line::styled(" No CLI registrations configured.", dim()));
        lines.push(Line::styled(" Press 'n' to add one.", dim()));
    } else {
        lines.push(Line::styled(
            format!(
                " {:<6} {:<18} {:<11} {}",
                "state", "name", "inject", "content"
            ),
            dim(),
        ));
        for (index, entry) in list.entries.iter().enumerate() {
            let selected = index == list.selected;
            let deleting = list.confirm_delete == Some(index);
            let style = if deleting {
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
            } else if selected {
                focus()
            } else {
                Style::default().fg(crate::theme::text())
            };
            let switch = if entry.enabled { "[ON]" } else { "[OFF]" };
            lines.push(Line::from(vec![
                Span::styled(
                    format!(" {:<6} ", switch),
                    if entry.enabled {
                        Style::default().fg(Color::Green)
                    } else {
                        dim()
                    },
                ),
                Span::styled(format!("{:<18} ", entry.name), style),
                Span::styled(format!("{:<11} ", entry.inject_to.label()), style),
                Span::styled(entry.summary(), style),
            ]));
        }
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(crate::theme::rounded_block_plain().title(title))
            .alignment(Alignment::Left),
        area,
    );
}

fn field(label: &str, value: String, selected: bool, hint: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!(" {label:<10}"), dim()),
        Span::styled(
            value,
            if selected {
                focus()
            } else {
                Style::default().fg(crate::theme::text())
            },
        ),
        Span::styled(
            if selected {
                format!("  {hint}")
            } else {
                String::new()
            },
            dim(),
        ),
    ])
}

fn render_form(frame: &mut Frame, area: Rect, top: u16, form: &CliForm) {
    let area = popup(area, top, 8);
    frame.render_widget(Clear, area);
    let mode = if form.original_name.is_some() {
        "edit"
    } else {
        "new"
    };
    let content = form.display_content();
    let lines = vec![
        field(
            "name:",
            if form.name.is_empty() {
                "(empty)".into()
            } else {
                form.name.clone()
            },
            form.field == CliField::Name,
            "type, Enter=save",
        ),
        field(
            "enabled:",
            if form.enabled {
                "on".into()
            } else {
                "off".into()
            },
            form.field == CliField::Enabled,
            "Space/Enter toggle",
        ),
        field(
            "inject to:",
            form.inject_to.label(),
            form.field == CliField::InjectTo,
            "Space/Enter pick",
        ),
        field(
            "content:",
            if content.is_empty() {
                "(empty)".into()
            } else {
                content
            },
            form.field == CliField::Content,
            "Enter multiline editor",
        ),
    ];
    let title = format!(" /cli {mode} — Tab/↑/↓ field, ←/→ cursor, Enter save, Esc cancel ");
    frame.render_widget(
        Paragraph::new(lines).block(crate::theme::rounded_block_plain().title(title)),
        area,
    );
    let (row, cursor) = match form.field {
        CliField::Name => (Some(0), form.name_cursor),
        CliField::Content => {
            let prefix: String = form.content.chars().take(form.content_cursor).collect();
            (
                Some(3),
                prefix
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .chars()
                    .count(),
            )
        }
        CliField::Enabled | CliField::InjectTo => (None, 0),
    };
    if let Some(row) = row {
        frame.set_cursor_position((area.x + 1 + 11 + cursor as u16, area.y + 1 + row));
    }
}

// ── multi-line content editor overlay ──────────────────────────────────────

/// Centered overlay rect for the editor: title + VIEW_LINES + borders.
fn content_dialog_area(area: Rect) -> Rect {
    let h = (VIEW_LINES as u16 + 2).min(area.height.saturating_sub(1).max(1));
    let w = 72u16.min(area.width.saturating_sub(2));
    Rect::new(
        area.x + area.width.saturating_sub(w) / 2,
        area.y + area.height.saturating_sub(h) / 2,
        w,
        h,
    )
}

/// Render the multi-line editor over `area` (the full popup surface) and
/// place the terminal cursor at the dialog's logical cursor position.
pub fn render_content_dialog(frame: &mut Frame, area: Rect, dialog: &ContentDialog) {
    let popup = content_dialog_area(area);
    frame.render_widget(Clear, popup);
    let (cursor_line, cursor_col) = dialog.line_col();
    let inner_w = popup.width.saturating_sub(2) as usize;
    let lines: Vec<Line> = dialog
        .text
        .split('\n')
        .skip(dialog.scroll)
        .take(VIEW_LINES)
        .map(|l| {
            let shown: String = l.chars().take(inner_w).collect();
            Line::styled(
                format!(" {shown}"),
                Style::default().fg(crate::theme::text()),
            )
        })
        .collect();
    let title = " content — Enter newline, Ctrl+S apply, Esc cancel ";
    frame.render_widget(
        Paragraph::new(lines).block(crate::theme::rounded_block_plain().title(title)),
        popup,
    );
    let visible_line = cursor_line.saturating_sub(dialog.scroll) as u16;
    if visible_line < VIEW_LINES as u16 {
        let col = (cursor_col as u16).min(inner_w.saturating_sub(1) as u16);
        frame.set_cursor_position((popup.x + 1 + col, popup.y + 1 + visible_line));
    }
}
