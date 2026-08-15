//! Rendering for the `/mcp` modal (mirrors `/model` view patterns).

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

use super::form::{McpField, McpForm};
use super::list::McpList;
use super::state::McpMenu;

/// Dispatch to the correct renderer based on the modal variant.
pub fn render_mcp_popup(f: &mut Frame, area: Rect, composer_top: u16, menu: &McpMenu) {
    match menu {
        McpMenu::List(list) => render_list(f, area, composer_top, list),
        McpMenu::Form(form) => render_form(f, area, composer_top, form),
    }
}

fn focus_style() -> Style {
    Style::default()
        .fg(crate::theme::warn_color())
        .add_modifier(Modifier::BOLD)
}

fn dim_style() -> Style {
    Style::default().fg(crate::theme::subtle())
}

fn val_style() -> Style {
    Style::default().fg(crate::theme::text())
}

fn field_line(label: &str, value: &str, focused: bool, hint: &str) -> Line<'static> {
    let mut spans = vec![
        Span::styled(format!(" {label:<14}"), dim_style()),
        Span::styled(
            value.to_string(),
            if focused { focus_style() } else { val_style() },
        ),
    ];
    if focused {
        spans.push(Span::styled(
            format!("  {hint}"),
            Style::default().fg(crate::theme::muted()),
        ));
    }
    Line::from(spans)
}

// ── server list ───────────────────────────────────────────────────────────

fn render_list(f: &mut Frame, area: Rect, composer_top: u16, list: &McpList) {
    let n = list.entries.len() as u16;
    let want_h = n.max(4) + 5;
    let h = want_h.min(22u16).min(composer_top.max(1));
    let w = 76u16.min(area.width.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = composer_top.saturating_sub(h);
    let popup = Rect::new(x, y, w, h);
    f.render_widget(Clear, popup);

    let title = match list.confirm_delete {
        Some(_) => " /mcp \u{2014} CONFIRM DELETE? y=delete, n/Esc=cancel ".to_string(),
        None => " /mcp \u{2014} \u{2191}/\u{2193} select, \u{2190}/\u{2192} toggle, e=edit, n=new, d=delete, Enter/Esc close ".to_string(),
    };
    let block = crate::theme::rounded_block_plain().title(title);

    let mut lines: Vec<Line> = Vec::new();
    if list.entries.is_empty() {
        lines.push(Line::styled(
            " No MCP servers configured.",
            Style::default().fg(crate::theme::warn_color()),
        ));
        lines.push(Line::styled(
            " Press 'n' to add one, or edit opencoder.json.",
            dim_style(),
        ));
    } else {
        lines.push(Line::styled(
            format!(
                " {:<5} {:<13} {:<11} {}",
                "on", "server", "inject", "transport"
            ),
            Style::default().fg(crate::theme::muted()),
        ));
        for (i, entry) in list.entries.iter().enumerate() {
            let selected = i == list.selected;
            let confirming = list.confirm_delete == Some(i);
            let switch = if entry.enabled { "[ON]" } else { "[OFF]" };
            let prefix = if confirming { "?" } else { " " };
            let line_style = if confirming {
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
            } else if selected {
                focus_style()
            } else if entry.enabled {
                Style::default().fg(crate::theme::accent())
            } else {
                val_style()
            };
            // The switch token carries its own state color so the toggle reads at a glance.
            let switch_style = if confirming {
                line_style
            } else if entry.enabled {
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD)
            } else {
                dim_style()
            };
            let spans = vec![
                Span::styled(format!("{}{:<5} ", prefix, switch), switch_style),
                Span::styled(format!("{:<13} ", entry.name), line_style),
                Span::styled(format!("{:<11} ", entry.inject_to.label()), line_style),
                Span::styled(entry.transport_label(), line_style),
            ];
            lines.push(Line::from(spans));
        }
    }

    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .alignment(Alignment::Left),
        popup,
    );
}

// ── add/edit form ─────────────────────────────────────────────────────────

fn render_form(f: &mut Frame, area: Rect, composer_top: u16, form: &McpForm) {
    let want_h = 10u16;
    let h = want_h.min(composer_top.max(1));
    let w = 76u16.min(area.width.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = composer_top.saturating_sub(h);
    let popup = Rect::new(x, y, w, h);
    f.render_widget(Clear, popup);

    let mode = if form.original_name.is_some() {
        "edit"
    } else {
        "new"
    };
    let title = format!(
        " /mcp {mode} \u{2014} type, \u{2190}/\u{2192} cursor, Enter=next/save, Space toggles enabled, Esc cancel "
    );
    let block = crate::theme::rounded_block_plain().title(title);

    let name_display = if form.name.is_empty() {
        "(empty)".to_string()
    } else {
        form.name.clone()
    };
    let cmd_display = if form.command.is_empty() {
        "(empty)".to_string()
    } else {
        form.command.clone()
    };
    let args_display = if form.args.is_empty() {
        "(empty)".to_string()
    } else {
        form.args.clone()
    };
    let url_display = if form.url.is_empty() {
        "(empty)".to_string()
    } else {
        form.url.clone()
    };
    let enabled_display = if form.enabled { "on" } else { "off" };

    let lines = vec![
        field_line(
            "name:",
            &name_display,
            form.field == McpField::Name,
            "type, \u{2190}/\u{2192} cursor, Enter=next",
        ),
        field_line(
            "enabled:",
            enabled_display,
            form.field == McpField::Enabled,
            "Space/Enter toggle",
        ),
        field_line(
            "inject to:",
            form.inject_to.label(),
            form.field == McpField::InjectTo,
            "Space/Enter cycle: parent/subagents/all",
        ),
        field_line(
            "command:",
            &cmd_display,
            form.field == McpField::Command,
            "stdio executable, \u{2190}/\u{2192} cursor",
        ),
        field_line(
            "args:",
            &args_display,
            form.field == McpField::Args,
            "space-separated, \u{2190}/\u{2192} cursor",
        ),
        field_line(
            "url:",
            &url_display,
            form.field == McpField::Url,
            "sse endpoint, \u{2190}/\u{2192} cursor",
        ),
    ];

    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .alignment(Alignment::Left),
        popup,
    );

    // Place terminal cursor at the edit position inside the focused text field.
    // Value column starts at inner x + 15 (1 leading space + 14-wide label).
    let text_field = match form.field {
        McpField::Name => Some(form.name.as_str()),
        McpField::Command => Some(form.command.as_str()),
        McpField::Args => Some(form.args.as_str()),
        McpField::Url => Some(form.url.as_str()),
        McpField::Enabled => None,
        McpField::InjectTo => None,
    };
    let cursor_idx = match form.field {
        McpField::Name => Some(form.name_cursor),
        McpField::Command => Some(form.command_cursor),
        McpField::Args => Some(form.args_cursor),
        McpField::Url => Some(form.url_cursor),
        McpField::Enabled => None,
        McpField::InjectTo => None,
    };
    let row = match form.field {
        McpField::Name => Some(0u16),
        McpField::Enabled => Some(1),
        McpField::InjectTo => Some(2),
        McpField::Command => Some(3),
        McpField::Args => Some(4),
        McpField::Url => Some(5),
    };
    if let (Some(raw), Some(idx), Some(row)) = (text_field, cursor_idx, row) {
        let cx = popup.x + 1 + 15 + crate::composer::cursor_column(raw, idx);
        let cy = popup.y + 1 + row;
        f.set_cursor_position((cx, cy));
    }
}
