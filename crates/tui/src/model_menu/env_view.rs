//! Rendering for the local Env manager.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};
use ratatui::Frame;

use super::env::EnvMenu;

pub fn render(frame: &mut Frame, area: Rect, composer_top: u16, menu: &EnvMenu) {
    let height = (menu.entries.len() as u16 + 10)
        .clamp(10, 22)
        .min(composer_top.max(1));
    let width = 78u16.min(area.width.saturating_sub(4));
    let popup = Rect::new(
        area.x + (area.width.saturating_sub(width)) / 2,
        composer_top.saturating_sub(height),
        width,
        height,
    );
    frame.render_widget(Clear, popup);

    let mut lines = vec![Line::styled(
        "Local configuration Envs",
        Style::default().add_modifier(Modifier::BOLD),
    )];
    if menu.entries.is_empty() {
        lines.push(Line::styled(
            "No Envs. Press n to create one.",
            Style::default().fg(crate::theme::subtle()),
        ));
    }
    for (index, env) in menu.entries.iter().enumerate() {
        let selected = index == menu.selected;
        let active = if env.active { "*" } else { " " };
        let marker = if selected { "❯" } else { " " };
        let style = if selected {
            Style::default()
                .fg(crate::theme::warn_color())
                .add_modifier(Modifier::BOLD)
        } else if env.active {
            Style::default().fg(crate::theme::accent())
        } else {
            Style::default().fg(crate::theme::text())
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{marker}{active} {:<24}", env.name), style),
            Span::styled(
                env.path.display().to_string(),
                Style::default().fg(crate::theme::subtle()),
            ),
        ]));
    }
    if let Some(input) = &menu.create_input {
        lines.push(Line::raw(format!("New Env name: {input}_")));
    }
    if let Some(index) = menu.confirm_delete {
        if let Some(env) = menu.entries.get(index) {
            lines.push(Line::styled(
                format!("Delete Env `{}`? y/Enter delete · n/Esc cancel", env.name),
                Style::default().fg(crate::theme::err_color()),
            ));
        }
    }
    if menu.stale_marker {
        lines.push(Line::styled(
            "The active Env marker is stale and is being ignored.",
            Style::default().fg(crate::theme::warn_color()),
        ));
    }
    if let Some(notice) = &menu.notice {
        lines.push(Line::styled(
            notice.clone(),
            Style::default().fg(crate::theme::ok_color()),
        ));
    }
    if let Some(error) = &menu.error {
        lines.push(Line::styled(
            error.clone(),
            Style::default().fg(crate::theme::err_color()),
        ));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "↑/↓ select · Enter use · n new · o no Env · d delete · Esc close",
        Style::default().fg(crate::theme::accent()),
    ));
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                ratatui::widgets::Block::bordered()
                    .title(" /envs ")
                    .border_style(Style::default().fg(crate::theme::accent())),
            )
            .wrap(Wrap { trim: false }),
        popup,
    );
}
