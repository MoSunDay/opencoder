//! Rendering for the scoped Act/Plan generation form.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

use super::mode_config_form::{ModeConfigForm, ModeField};

fn bool_label(value: Option<bool>, inherited: Option<bool>) -> String {
    match value {
        Some(value) => if value {
            "on (override)"
        } else {
            "off (override)"
        }
        .to_string(),
        None => format!(
            "inherit Shared ({})",
            if inherited.unwrap_or(false) {
                "on"
            } else {
                "off"
            }
        ),
    }
}

fn row<'a>(label: &str, value: String, focused: bool) -> Line<'a> {
    Line::from(vec![
        Span::styled(
            format!(" {label:<22}"),
            Style::default().fg(crate::theme::subtle()),
        ),
        Span::styled(
            value,
            if focused {
                Style::default()
                    .fg(crate::theme::warn_color())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(crate::theme::text())
            },
        ),
    ])
}

pub(crate) fn render(f: &mut Frame, area: Rect, composer_top: u16, form: &ModeConfigForm) {
    let h = 17u16.min(composer_top.max(1));
    let w = 84u16.min(area.width.saturating_sub(4));
    let popup = Rect::new(
        area.x + (area.width.saturating_sub(w)) / 2,
        composer_top.saturating_sub(h),
        w,
        h,
    );
    f.render_widget(Clear, popup);
    let inherited = form.inherited();
    let reasoning = if form.reasoning_inherit {
        format!(
            "inherit Shared ({})",
            inherited.reasoning_effort.as_deref().unwrap_or("default")
        )
    } else {
        format!("{} (override)", form.reasoning.label())
    };
    let max_tokens = if form.max_tokens_input.is_empty() {
        format!(
            "inherit Shared ({})",
            inherited
                .max_tokens
                .map(|value| value.to_string())
                .unwrap_or_else(|| "default".into())
        )
    } else {
        form.max_tokens_input.clone()
    };
    let context_limit = if form.context_limit_input.is_empty() {
        format!("inherit Shared ({})", inherited.context_limit())
    } else {
        form.context_limit_input.clone()
    };
    let mut lines = vec![
        Line::styled(
            format!(
                "{} generation overrides",
                crate::model_scope::mode_name(form.mode)
            ),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        row("reasoning:", reasoning, form.focus == ModeField::Reasoning),
        row(
            "interleaved_thinking:",
            bool_label(form.interleaved_thinking, inherited.interleaved_thinking),
            form.focus == ModeField::InterleavedThinking,
        ),
        row(
            "max_tokens:",
            max_tokens,
            form.focus == ModeField::MaxTokens,
        ),
        row(
            "context_limit:",
            context_limit,
            form.focus == ModeField::ContextLimit,
        ),
        row(
            "cache_salt:",
            bool_label(form.cache_salt, inherited.cache_salt),
            form.focus == ModeField::CacheSalt,
        ),
        Line::raw(""),
        row(
            "Save / Cancel:",
            match form.focus {
                ModeField::Save => "[ Save ]   Cancel".to_string(),
                ModeField::Cancel => "  Save   [ Cancel ]".to_string(),
                _ => "  Save     Cancel".to_string(),
            },
            matches!(form.focus, ModeField::Save | ModeField::Cancel),
        ),
        Line::raw(format!(
            "Active Env: {}",
            form.source.save_target.env_label()
        )),
        Line::raw(format!("Save target: {}", form.source.save_target.path)),
        Line::styled(
            "Enter next/save · Space change · i/Delete inherit · Esc back",
            Style::default().fg(crate::theme::accent()),
        ),
    ];
    if let Some(error) = &form.error {
        lines.push(Line::styled(
            error.clone(),
            Style::default().fg(crate::theme::err_color()),
        ));
    }
    f.render_widget(
        Paragraph::new(lines).block(
            ratatui::widgets::Block::bordered()
                .title(format!(
                    " /config {} ",
                    crate::model_scope::mode_name(form.mode)
                ))
                .border_style(Style::default().fg(crate::theme::accent())),
        ),
        popup,
    );
}
