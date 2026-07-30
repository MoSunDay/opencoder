//! Rendering for `/config` and `/model` modals.

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
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

// ── /config form ──────────────────────────────────────────────────────────

fn render_config_form(f: &mut Frame, area: Rect, composer_top: u16, form: &ConfigForm) {
    let want_h = 17u16;
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
    let block = crate::theme::rounded_block_plain().title(title);

    let threshold_hint = match form.threshold_input.trim().parse::<u64>() {
        Ok(v) => format!("{} tokens (\u{2248}{}k)", v, v / 1000),
        Err(_) => "(empty)".to_string(),
    };
    let context_size_hint = match form.context_size_input.trim().parse::<u64>() {
        Ok(v) => format!("{} tokens (\u{2248}{}k)", v, v / 1000),
        Err(_) => "(empty)".to_string(),
    };
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
            "ctx size:",
            &context_size_hint,
            form.focus == ConfigField::ContextSize,
            "digits/\u{2190}\u{2192} \u{00b1}1k, Backspace, Enter=next",
        ),
        field_line(
            "ctx threshold:",
            &threshold_hint,
            form.focus == ConfigField::Threshold,
            "digits/\u{2190}\u{2192} \u{00b1}1k, Backspace, Enter=next",
        ),
        field_line(
            "fps:",
            &if form.fps_input.is_empty() {
                "(empty)".to_string()
            } else {
                format!("{} FPS", form.fps_input)
            },
            form.focus == ConfigField::Fps,
            "1-30, digits/\u{2190}\u{2192} \u{00b1}1, Backspace",
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
        field_line(
            "autopilot:",
            if form.ap_enabled { "[ on ]" } else { "[ off ]" },
            form.focus == ConfigField::ApEnabled,
            "\u{2190}/\u{2192}/Space toggle",
        ),
        field_line(
            "ap max_iter:",
            &if form.ap_max_iter_input.is_empty() {
                "(empty)".to_string()
            } else {
                form.ap_max_iter_input.clone()
            },
            form.focus == ConfigField::ApMaxIter,
            "1+, digits/\u{2190}/\u{2192} \u{00b1}1, Backspace",
        ),
        field_line(
            "theme:",
            &format!("[ {} ]", form.theme.label()),
            form.focus == ConfigField::Theme,
            "\u{2190}/\u{2192}/Space cycle",
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
    // Place terminal cursor at end of raw input for text fields.
    if let Some(row) = text_field_row(form.focus) {
        if let Some(raw) = focused_raw_input(form) {
            let cx = popup.x + 1 + 15 + raw.chars().count() as u16;
            let cy = popup.y + 1 + row as u16;
            f.set_cursor_position((cx, cy));
        }
    }
}

/// Row index in the config-form `lines` vec for text-edit fields (0-based).
fn text_field_row(field: ConfigField) -> Option<usize> {
    match field {
        ConfigField::MaxTokens => Some(2),
        ConfigField::ContextSize => Some(3),
        ConfigField::Threshold => Some(4),
        ConfigField::Fps => Some(5),
        ConfigField::ApMaxIter => Some(10),
        _ => None,
    }
}

/// Raw input buffer for the currently-focused text field, if any.
fn focused_raw_input(form: &ConfigForm) -> Option<&str> {
    match form.focus {
        ConfigField::MaxTokens => Some(&form.max_tokens_input),
        ConfigField::ContextSize => Some(&form.context_size_input),
        ConfigField::Threshold => Some(&form.threshold_input),
        ConfigField::Fps => Some(&form.fps_input),
        ConfigField::ApMaxIter => Some(&form.ap_max_iter_input),
        _ => None,
    }
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

    let title = if list.confirm_save_default.is_some() {
        " /model \u{2014} SAVE AS DEFAULT? y/Enter=global, n=session-only ".to_string()
    } else {
        match &list.confirm_delete {
            Some(_) => " /model \u{2014} CONFIRM DELETE? y=delete, n/Esc=cancel ".to_string(),
            None => " /model \u{2014} \u{2191}/\u{2193} select, Enter=switch, e=edit, n=new, d=delete, Esc cancel ".to_string(),
        }
    };
    let block = crate::theme::rounded_block_plain().title(title);

    let mut lines: Vec<Line> = Vec::new();
    if list.entries.is_empty() {
        lines.push(Line::styled(
            " No providers configured.",
            Style::default().fg(crate::theme::warn_color()),
        ));
        lines.push(Line::styled(
            " Press 'n' to add one, or edit opencoder.json.",
            dim_style(),
        ));
    } else {
        lines.push(Line::styled(
            format!(" {:<14} {:<30} {}", "provider", "base_url", "model"),
            Style::default().fg(crate::theme::muted()),
        ));
        for (i, entry) in list.entries.iter().enumerate() {
            let selected = i == list.selected;
            let confirming = list.confirm_delete == Some(i);
            let asking_default = list.confirm_save_default.is_some() && selected;
            let mark = if entry.active { "\u{25cf}" } else { " " };
            let prefix = if confirming || asking_default {
                "?"
            } else {
                " "
            };
            let model_display = if entry.model_id.is_empty() {
                "(unset)"
            } else {
                entry.model_id.as_str()
            };
            let text = format!(
                "{}{} {:<13} {:<30} {}",
                prefix, mark, entry.name, entry.base_url, model_display
            );
            let style = if confirming {
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
            } else if asking_default {
                Style::default()
                    .fg(crate::theme::warn_color())
                    .add_modifier(Modifier::BOLD)
            } else if selected {
                focus_style()
            } else if entry.active {
                Style::default().fg(crate::theme::accent())
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

    if list.confirm_save_default.is_some() {
        render_save_default_confirm(f, area, list);
    }
}

/// Centered overlay shown when the user has selected a model and we are asking
/// whether to save it as the global default (vs session-only). Drawn on top of
/// the provider-list popup so the prompt is unmistakable.
fn render_save_default_confirm(f: &mut Frame, area: Rect, list: &ProviderList) {
    let w = 62u16.min(area.width.saturating_sub(4));
    let h = 5u16;
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let popup = Rect::new(x, y, w, h);
    f.render_widget(Clear, popup);

    let (name, model) = list
        .entries
        .get(list.selected)
        .map(|e| {
            (
                e.name.as_str(),
                if e.model_id.is_empty() {
                    "(unset)"
                } else {
                    e.model_id.as_str()
                },
            )
        })
        .unwrap_or(("?", "?"));

    let title = " Save as default? ".to_string();
    let lines = vec![
        Line::styled(
            format!(" Set {}/{} as global default? ", name, model),
            Style::default()
                .fg(crate::theme::warn_color())
                .add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
        Line::styled(
            " [y]/Enter global    [n] session-only    Esc cancel ",
            Style::default().fg(crate::theme::accent()),
        ),
    ];
    f.render_widget(
        Paragraph::new(lines)
            .block(
                crate::theme::rounded_block_plain()
                    .title(title)
                    .border_style(Style::default().fg(crate::theme::warn_color())),
            )
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
    let block = crate::theme::rounded_block_plain().title(title);

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
                Style::default().fg(crate::theme::muted()),
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
