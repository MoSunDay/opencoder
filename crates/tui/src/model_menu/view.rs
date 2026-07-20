//! Rendering for the `/config` modal. State lives in [`super::state`].

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use super::state::{Field, ModelMenu, ModelMenuMode};

/// Draw the `/config` modal as a dropdown overlay anchored above the composer.
///
/// `composer_top` is the screen row of the composer's top border; the popup's
/// bottom edge sits just above it, so the form floats over the transcript like
/// a dropdown instead of covering the screen center.
pub fn render_model_popup(f: &mut Frame, area: Rect, composer_top: u16, menu: &ModelMenu) {
    if menu.mode == ModelMenuMode::ProviderList {
        render_provider_list_popup(f, area, composer_top, menu);
        return;
    }
    let want_h = 21u16;
    let h = want_h.min(composer_top.max(1));
    let w = 72u16.min(area.width.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = composer_top.saturating_sub(h);
    let popup = Rect::new(x, y, w, h);
    f.render_widget(Clear, popup);

    let title = match &menu.error {
        Some(e) => format!(" /config \u{2014} ERROR: {e} "),
        None => " /config \u{2014} \u{2191}/\u{2193} option, \u{2190}/\u{2192} change value, Enter=next, [Save] commits, Esc cancel ".to_string(),
    };
    let block = Block::default().borders(Borders::ALL).title(title);

    let focus_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(Color::Gray);
    let val_style = Style::default().fg(Color::White);

    let field = |label: &str, value: &str, focused: bool, hint: &str| -> Line<'_> {
        let mut spans = vec![
            Span::styled(format!(" {label:<14}"), dim),
            Span::styled(
                value.to_string(),
                if focused { focus_style } else { val_style },
            ),
        ];
        if focused {
            spans.push(Span::styled(
                format!("  {hint}"),
                Style::default().fg(Color::DarkGray),
            ));
        }
        Line::from(spans)
    };

    let threshold_hint = format!(
        "{} tokens (\u{2248}{}k)",
        menu.threshold,
        menu.threshold / 1000
    );
    let reasoning_val = format!("[ {} ]", menu.reasoning.label());
    let interleave_val = format!(
        "[ {} ]",
        if menu.interleaved_thinking {
            "on"
        } else {
            "off"
        }
    );

    let lines = vec![
        field(
            "model:",
            menu.model.as_str(),
            menu.focus == Field::Model,
            "type, Enter=next",
        ),
        field(
            "base_url:",
            menu.base_url.as_str(),
            menu.focus == Field::BaseUrl,
            "type, Enter=next",
        ),
        field(
            "api_key:",
            menu.api_key_display().as_str(),
            menu.focus == Field::ApiKey,
            "type new value, Enter=next",
        ),
        field(
            "thinking:",
            reasoning_val.as_str(),
            menu.focus == Field::Reasoning,
            "\u{2190}/\u{2192}/Space cycle, Enter=next",
        ),
        field(
            "interleave:",
            interleave_val.as_str(),
            menu.focus == Field::InterleavedThinking,
            "\u{2190}/\u{2192}/Space toggle, Enter=next",
        ),
        field(
            "ctx threshold:",
            threshold_hint.as_str(),
            menu.focus == Field::Threshold,
            "digits/\u{2190}\u{2192} \u{00b1}1k, Enter=next",
        ),
        field(
            "fps:",
            format!("{} FPS", menu.fps).as_str(),
            menu.focus == Field::Fps,
            "1-30, digits/\u{2190}\u{2192} \u{00b1}1, higher = more CPU (10 = smooth)",
        ),
        field(
            "browser:",
            format!(
                "[ {} ]",
                if menu.capabilities_browser { "on" } else { "off" }
            )
            .as_str(),
            menu.focus == Field::Browser,
            "\u{2190}/\u{2192}/Space toggle (web_fetch/web_search), Enter=next",
        ),
        field(
            "computer_use:",
            format!(
                "[ {} ]",
                if menu.capabilities_computer_use { "on" } else { "off" }
            )
            .as_str(),
            menu.focus == Field::ComputerUse,
            "\u{2190}/\u{2192}/Space toggle, Enter=next",
        ),
        field(
            "tools_subagent:",
            format!(
                "[ {} ]",
                if menu.capabilities_tools_subagent { "on" } else { "off" }
            )
            .as_str(),
            menu.focus == Field::ToolsSubagent,
            "\u{2190}/\u{2192}/Space toggle, Enter=next",
        ),
        Line::from(""),
        button_line(menu),
    ];

    let para = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .alignment(Alignment::Left);
    f.render_widget(para, popup);
}

fn button_line(menu: &ModelMenu) -> Line<'_> {
    let save_style = if menu.focus == Field::Save {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Green)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Green)
    };
    let cancel_style = if menu.focus == Field::Cancel {
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

/// Render the provider-list selector overlay (`/model`).
fn render_provider_list_popup(f: &mut Frame, area: Rect, composer_top: u16, menu: &ModelMenu) {
    let n = menu.provider_entries.len() as u16;
    let want_h = n.max(5) + 4;
    let h = want_h.min(20u16).min(composer_top.max(1));
    let w = 76u16.min(area.width.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = composer_top.saturating_sub(h);
    let popup = Rect::new(x, y, w, h);
    f.render_widget(Clear, popup);

    let title = match &menu.error {
        Some(e) => format!(" /model \u{2014} ERROR: {e} "),
        None => " /model \u{2014} \u{2191}/\u{2193} select, Enter=switch, Esc cancel ".to_string(),
    };
    let block = Block::default().borders(Borders::ALL).title(title);

    let focus_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(Color::Gray);
    let active_style = Style::default().fg(Color::Cyan);

    let mut lines: Vec<Line> = Vec::new();
    if menu.provider_entries.is_empty() {
        lines.push(Line::styled(
            " No providers configured.",
            Style::default().fg(Color::Yellow),
        ));
        lines.push(Line::styled(
            " Add entries to the `providers` map in opencoder.json:",
            dim,
        ));
        lines.push(Line::styled(
            "   \"providers\": {",
            Style::default().fg(Color::DarkGray),
        ));
        lines.push(Line::styled(
            "     \"deepseek\": { \"base_url\": \"...\", \"model\": \"deepseek-chat\" }",
            Style::default().fg(Color::DarkGray),
        ));
        lines.push(Line::styled("   }", Style::default().fg(Color::DarkGray)));
    } else {
        // Header row.
        lines.push(Line::styled(
            format!(
                " {:<14} {:<34} {}",
                "provider", "base_url", "model"
            ),
            Style::default().fg(Color::DarkGray),
        ));
        for (i, entry) in menu.provider_entries.iter().enumerate() {
            let selected = i == menu.provider_selected;
            let mark = if entry.active { "\u{25cf}" } else { " " };
            let text = format!(
                " {} {:<13} {:<34} {}",
                mark, entry.name, entry.base_url, entry.model_id
            );
            let style = if selected {
                focus_style
            } else if entry.active {
                active_style
            } else {
                Style::default().fg(Color::White)
            };
            lines.push(Line::styled(text, style));
        }
    }

    let para = Paragraph::new(lines)
        .block(block)
        .alignment(Alignment::Left);
    f.render_widget(para, popup);
}
