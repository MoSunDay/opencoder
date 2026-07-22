//! Rendering for `/config` and `/model` modals.

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use super::config_form::{ConfigField, ConfigForm};
use super::list::ProviderList;
use super::provider_form::{ProviderField, ProviderForm};
use super::state::ModelMenu;

/// Dispatch to the correct renderer based on the modal variant.
pub fn render_model_popup(f: &mut Frame, area: Rect, composer_top: u16, menu: &ModelMenu) {
    match menu {
        ModelMenu::Config(form) => render_config_form(f, area, composer_top, form),
        ModelMenu::List(list) => render_provider_list(f, area, composer_top, list),
        ModelMenu::Form(form) => render_provider_form(f, area, composer_top, form),
    }
}

fn focus_style() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD)
}

fn dim_style() -> Style {
    Style::default().fg(Color::Gray)
}

fn val_style() -> Style {
    Style::default().fg(Color::White)
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
            Style::default().fg(Color::DarkGray),
        ));
    }
    Line::from(spans)
}

// ── /config form ──────────────────────────────────────────────────────────

fn render_config_form(f: &mut Frame, area: Rect, composer_top: u16, form: &ConfigForm) {
    let want_h = 16u16;
    let h = want_h.min(composer_top.max(1));
    let w = 72u16.min(area.width.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = composer_top.saturating_sub(h);
    let popup = Rect::new(x, y, w, h);
    f.render_widget(Clear, popup);

    let title = match &form.error {
        Some(e) => format!(" /config \u{2014} ERROR: {e} "),
        None => " /config \u{2014} \u{2191}/\u{2193} option, \u{2190}/\u{2192} change, Enter=next, [Save] commits, Esc cancel ".to_string(),
    };
    let block = Block::default().borders(Borders::ALL).title(title);

    let threshold_hint = format!(
        "{} tokens (\u{2248}{}k)",
        form.threshold,
        form.threshold / 1000
    );
    let reasoning_val = format!("[ {} ]", form.reasoning.label());
    let interleave_val = format!(
        "[ {} ]",
        if form.interleaved_thinking {
            "on"
        } else {
            "off"
        }
    );
    let max_tokens_val = if form.max_tokens_input.is_empty() {
        "(unset)".to_string()
    } else {
        form.max_tokens_input.clone()
    };

    let lines = vec![
        field_line(
            "thinking:",
            &reasoning_val,
            form.focus == ConfigField::Reasoning,
            "\u{2190}/\u{2192}/Space cycle, Enter=next",
        ),
        field_line(
            "interleave:",
            &interleave_val,
            form.focus == ConfigField::InterleavedThinking,
            "\u{2190}/\u{2192}/Space toggle, Enter=next",
        ),
        field_line(
            "max_tokens:",
            &max_tokens_val,
            form.focus == ConfigField::MaxTokens,
            "digits, Backspace, empty=unset, Enter=next",
        ),
        field_line(
            "ctx threshold:",
            &threshold_hint,
            form.focus == ConfigField::Threshold,
            "digits/\u{2190}\u{2192} \u{00b1}1k, Enter=next",
        ),
        field_line(
            "fps:",
            &format!("{} FPS", form.fps),
            form.focus == ConfigField::Fps,
            "1-30, digits/\u{2190}\u{2192} \u{00b1}1",
        ),
        field_line(
            "browser:",
            &format!(
                "[ {} ]",
                if form.capabilities_browser {
                    "on"
                } else {
                    "off"
                }
            ),
            form.focus == ConfigField::Browser,
            "\u{2190}/\u{2192}/Space toggle",
        ),
        field_line(
            "computer_use:",
            &format!(
                "[ {} ]",
                if form.capabilities_computer_use {
                    "on"
                } else {
                    "off"
                }
            ),
            form.focus == ConfigField::ComputerUse,
            "\u{2190}/\u{2192}/Space toggle",
        ),
        field_line(
            "tools_sub:",
            &format!(
                "[ {} ]",
                if form.capabilities_tools_subagent {
                    "on"
                } else {
                    "off"
                }
            ),
            form.focus == ConfigField::ToolsSubagent,
            "\u{2190}/\u{2192}/Space toggle",
        ),
        button_line_cfg(form),
        Line::raw(""),
    ];

    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .alignment(Alignment::Left),
        popup,
    );
}

fn button_line_cfg(form: &ConfigForm) -> Line<'_> {
    let save_style = if form.focus == ConfigField::Save {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Green)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Green)
    };
    let cancel_style = if form.focus == ConfigField::Cancel {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Red)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Red)
    };
    Line::from(vec![
        Span::raw("   "),
        Span::styled("[ Save ]", save_style),
        Span::raw("    "),
        Span::styled("[ Cancel ]", cancel_style),
    ])
}

// ── /model provider list ──────────────────────────────────────────────────

fn render_provider_list(f: &mut Frame, area: Rect, composer_top: u16, list: &ProviderList) {
    let n = list.entries.len() as u16;
    let want_h = n.max(5) + 5;
    let h = want_h.min(22u16).min(composer_top.max(1));
    let w = 76u16.min(area.width.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = composer_top.saturating_sub(h);
    let popup = Rect::new(x, y, w, h);
    f.render_widget(Clear, popup);

    let title = match &list.confirm_delete {
        Some(_) => " /model \u{2014} CONFIRM DELETE? y=delete, n/Esc=cancel ".to_string(),
        None => " /model \u{2014} \u{2191}/\u{2193} select, Enter=switch, e=edit, n=new, d=delete, Esc cancel ".to_string(),
    };
    let block = Block::default().borders(Borders::ALL).title(title);

    let mut lines: Vec<Line> = Vec::new();
    if list.entries.is_empty() {
        lines.push(Line::styled(
            " No providers configured.",
            Style::default().fg(Color::Yellow),
        ));
        lines.push(Line::styled(
            " Press 'n' to add one, or edit opencoder.json.",
            dim_style(),
        ));
    } else {
        lines.push(Line::styled(
            format!(" {:<14} {:<30} {}", "provider", "base_url", "model"),
            Style::default().fg(Color::DarkGray),
        ));
        for (i, entry) in list.entries.iter().enumerate() {
            let selected = i == list.selected;
            let confirming = list.confirm_delete == Some(i);
            let mark = if entry.active { "\u{25cf}" } else { " " };
            let prefix = if confirming { "?" } else { " " };
            let text = format!(
                "{}{} {:<13} {:<30} {}",
                prefix, mark, entry.name, entry.base_url, entry.model_id
            );
            let style = if confirming {
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
            } else if selected {
                focus_style()
            } else if entry.active {
                Style::default().fg(Color::Cyan)
            } else {
                val_style()
            };
            lines.push(Line::styled(text, style));
        }
    }

    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .alignment(Alignment::Left),
        popup,
    );
}

// ── /model provider form ──────────────────────────────────────────────────

fn render_provider_form(f: &mut Frame, area: Rect, composer_top: u16, form: &ProviderForm) {
    let header_count = form.headers.pairs.len() as u16;
    let want_h = 11u16 + header_count.max(1);
    let h = want_h.min(composer_top.max(1));
    let w = 72u16.min(area.width.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = composer_top.saturating_sub(h);
    let popup = Rect::new(x, y, w, h);
    f.render_widget(Clear, popup);

    let mode = if form.name_readonly { "edit" } else { "new" };
    let title = match &form.error {
        Some(e) => format!(" /model {mode} \u{2014} ERROR: {e} "),
        None => {
            format!(" /model {mode} \u{2014} type values, Enter=next, [Save] commits, Esc cancel ")
        }
    };
    let block = Block::default().borders(Borders::ALL).title(title);

    let name_display = if form.name_readonly {
        format!("{} (read-only)", form.name)
    } else {
        if form.name.is_empty() {
            "(empty)".to_string()
        } else {
            form.name.clone()
        }
    };
    let model_display = if form.model_id.is_empty() {
        "(empty)".to_string()
    } else {
        form.model_id.clone()
    };
    let base_display = if form.base_url.is_empty() {
        "(empty)".to_string()
    } else {
        form.base_url.clone()
    };

    let mut lines = vec![
        field_line(
            "name:",
            &name_display,
            form.focus == ProviderField::Name && !form.name_readonly,
            "type provider name, Enter=next",
        ),
        field_line(
            "model_id:",
            &model_display,
            form.focus == ProviderField::ModelId,
            "type model id, Enter=next",
        ),
        field_line(
            "base_url:",
            &base_display,
            form.focus == ProviderField::BaseUrl,
            "type URL, Enter=next",
        ),
        field_line(
            "api_key:",
            &form.api_key_display(),
            form.focus == ProviderField::ApiKey,
            "type new value, Enter=next",
        ),
    ];

    // Headers section
    let hdr_hint = if form.headers_active {
        format!("[editing: pair {}/{}, {}] \u{2191}\u{2193}pair \u{2190}\u{2192}name/val +/-add/del, Enter=done",
            form.headers.selected + 1,
            form.headers.pairs.len().max(1),
            form.headers.active_label())
    } else {
        "Enter to edit".to_string()
    };
    lines.push(field_line(
        "headers:",
        &format!(
            "({} pair{})",
            form.headers.pairs.len(),
            if form.headers.pairs.len() == 1 {
                ""
            } else {
                "s"
            }
        ),
        form.focus == ProviderField::Headers,
        &hdr_hint,
    ));

    if form.headers_active || form.focus == ProviderField::Headers {
        if form.headers.pairs.is_empty() {
            lines.push(Line::styled(
                "     (no headers, press + to add)",
                Style::default().fg(Color::DarkGray),
            ));
        }
        for (i, (hn, hv)) in form.headers.pairs.iter().enumerate() {
            let selected = i == form.headers.selected;
            let name_focus = selected && form.headers_active && !form.headers.editing_value;
            let val_focus = selected && form.headers_active && form.headers.editing_value;
            let name_disp = if hn.is_empty() { "(name)" } else { hn.as_str() };
            let val_disp = if hv.is_empty() {
                "(value)"
            } else {
                hv.as_str()
            };
            let style = if selected && form.headers_active {
                focus_style()
            } else {
                dim_style()
            };
            let n_style = if name_focus { focus_style() } else { style };
            let v_style = if val_focus { focus_style() } else { style };
            lines.push(Line::from(vec![
                Span::raw("     "),
                Span::styled(format!("{:<20}", name_disp), n_style),
                Span::raw(" = "),
                Span::styled(val_disp.to_string(), v_style),
            ]));
        }
    }

    // Buttons
    let save_style = if form.focus == ProviderField::Save {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Green)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Green)
    };
    let cancel_style = if form.focus == ProviderField::Cancel {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Red)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Red)
    };
    lines.push(Line::from(vec![
        Span::raw("   "),
        Span::styled("[ Save ]", save_style),
        Span::raw("    "),
        Span::styled("[ Cancel ]", cancel_style),
    ]));

    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .alignment(Alignment::Left),
        popup,
    );
}
