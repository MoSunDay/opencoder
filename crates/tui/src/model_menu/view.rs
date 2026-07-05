//! Rendering for the `/model` modal. State lives in [`super::state`].

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use super::state::{Field, ModelMenu};

/// Draw the `/model` modal as a centered overlay.
pub fn render_model_popup(f: &mut Frame, area: Rect, menu: &ModelMenu) {
    let h = 16u16.min(area.height.saturating_sub(2));
    let w = 72u16.min(area.width.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let popup = Rect::new(x, y, w, h);
    f.render_widget(Clear, popup);

    let title = match &menu.error {
        Some(e) => format!(" /model \u{2014} ERROR: {e} "),
        None => " /model \u{2014} Tab move, Enter on [Save], Esc cancel ".to_string(),
    };
    let block = Block::default().borders(Borders::ALL).title(title);

    let focus_style = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(Color::Gray);
    let val_style = Style::default().fg(Color::White);

    let field = |label: &str, value: &str, focused: bool, hint: &str| -> Line<'_> {
        let mut spans = vec![
            Span::styled(format!(" {label:<14}"), dim),
            Span::styled(value.to_string(), if focused { focus_style } else { val_style }),
        ];
        if focused {
            spans.push(Span::styled(format!("  {hint}"), Style::default().fg(Color::DarkGray)));
        }
        Line::from(spans)
    };

    let threshold_hint = format!("{} tokens (\u{2248}{}k)", menu.threshold, menu.threshold / 1000);
    let reasoning_val = format!("[ {} ]", menu.reasoning.label());

    let lines = vec![
        field("model:", menu.model.as_str(), menu.focus == Field::Model, "type to edit"),
        field("base_url:", menu.base_url.as_str(), menu.focus == Field::BaseUrl, "type to edit"),
        field("api_key:", menu.api_key_display().as_str(), menu.focus == Field::ApiKey, "type new value (hidden)"),
        field("thinking:", reasoning_val.as_str(), menu.focus == Field::Reasoning, "\u{2190}/\u{2192} or Space to cycle"),
        field("ctx threshold:", threshold_hint.as_str(), menu.focus == Field::Threshold, "digits / \u{2191}\u{2193} \u{00b1}1k"),
        Line::from(""),
        button_line(menu),
    ];

    let para = Paragraph::new(lines).block(block).wrap(Wrap { trim: false }).alignment(Alignment::Left);
    f.render_widget(para, popup);
}

fn button_line(menu: &ModelMenu) -> Line<'_> {
    let save_style = if menu.focus == Field::Save {
        Style::default().fg(Color::Black).bg(Color::Green).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Green)
    };
    let cancel_style = if menu.focus == Field::Cancel {
        Style::default().fg(Color::Black).bg(Color::Red).add_modifier(Modifier::BOLD)
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
