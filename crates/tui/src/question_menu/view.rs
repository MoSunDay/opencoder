//! Question dialog rendering: a compact popup anchored to the composer's top
//! edge (same geometry recipe as the model/mcp menus).

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

use super::state::{QuestionFocus, QuestionMenu};
use crate::theme;

/// Render the question dialog with its bottom edge flush against
/// `composer_top`. Places the terminal cursor inside the custom box when it
/// has focus.
pub fn render_question_popup(f: &mut Frame, area: Rect, composer_top: u16, menu: &QuestionMenu) {
    let inner_w = 56usize;
    let q_lines = wrapped_lines(&menu.prompt.question, inner_w).max(1) as u16;
    let option_rows = menu.prompt.options.len().max(1) as u16;
    // border(2) + question + blank + options + custom row + blank + hint
    let want_h = 2 + q_lines + 1 + option_rows + 1 + 1 + 1;
    let h = want_h.min(composer_top.max(1));
    let w = 60u16.min(area.width.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = composer_top.saturating_sub(h);
    let popup = Rect::new(x, y, w, h);
    f.render_widget(Clear, popup);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::styled(
        menu.prompt.question.clone(),
        Style::default().fg(theme::text()).add_modifier(Modifier::BOLD),
    ));
    lines.push(Line::from(""));

    let custom_row = menu.custom_row();
    for (i, opt) in menu.prompt.options.iter().enumerate() {
        let selected = menu.focus == QuestionFocus::Options && menu.selected == i;
        lines.push(option_line(opt, selected));
    }
    lines.push(custom_line(menu, custom_row));

    lines.push(Line::from(""));
    lines.push(Line::styled(
        "↑↓ select · Enter answer · Tab custom · Esc skip",
        Style::default().fg(theme::muted()),
    ));

    let block = crate::theme::rounded_block_plain().title(Span::styled(
        " Question ",
        Style::default().fg(theme::warn_color()).add_modifier(Modifier::BOLD),
    ));
    let para = Paragraph::new(lines).block(block).wrap(ratatui::widgets::Wrap { trim: false });
    f.render_widget(para, popup);

    if menu.focus == QuestionFocus::Custom {
        // Row index: border(1) + question(q..) + blank(1) + options + custom.
        let custom_y = popup.y + 1 + q_lines + 1 + menu.prompt.options.len() as u16 + 1;
        let custom_x = popup.x + 2
            + crate::composer::cursor_column(&menu.custom_input, menu.custom_cursor);
        if custom_x < popup.x + popup.width.saturating_sub(1)
            && custom_y < popup.y + popup.height.saturating_sub(1)
        {
            f.set_cursor_position((custom_x, custom_y));
        }
    }
}

fn option_line(opt: &str, selected: bool) -> Line<'static> {
    let marker = if selected { "▸ " } else { "  " };
    let style = if selected {
        Style::default().fg(theme::accent()).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::text())
    };
    Line::from(Span::styled(format!("{marker}{opt}"), style))
}

fn custom_line(menu: &QuestionMenu, custom_row: usize) -> Line<'static> {
    let selected = menu.focus == QuestionFocus::Options && menu.selected == custom_row;
    let focused = menu.focus == QuestionFocus::Custom;
    let body = if menu.custom_input.is_empty() {
        "✎ custom answer…"
    } else {
        menu.custom_input.as_str()
    };
    let style = if focused {
        Style::default().fg(theme::warn_color()).add_modifier(Modifier::BOLD)
    } else if selected {
        Style::default().fg(theme::accent())
    } else {
        Style::default().fg(theme::muted())
    };
    Line::from(Span::styled(format!("✎ {body}"), style))
}

/// Crude word-wrap row estimate (the Paragraph uses real wrapping; this only
/// reserves enough vertical space).
fn wrapped_lines(text: &str, width: usize) -> usize {
    let mut lines = 1;
    let mut col = 0;
    for word in text.split_whitespace() {
        let w = word.chars().count() + 1;
        if col + w > width && col > 0 {
            lines += 1;
            col = w;
        } else {
            col += w;
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::question_menu::state::QuestionPrompt;
    use ratatui::backend::TestBackend;

    fn menu() -> QuestionMenu {
        QuestionMenu::new(QuestionPrompt {
            id: "q-9".into(),
            question: "Which database engine should the migration target?".into(),
            options: vec!["sqlite".into(), "postgres".into()],
        })
    }

    fn rendered_text(custom_focus: bool) -> String {
        let mut m = menu();
        if custom_focus {
            crate::question_menu::state::handle_question_key(
                &mut m,
                crossterm::event::KeyEvent::new(crossterm::event::KeyCode::Tab, crossterm::event::KeyModifiers::NONE),
            );
        }
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render_question_popup(f, area, 20, &m);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        let mut out = String::new();
        for y in 0..24 {
            for x in 0..80 {
                if let Some(c) = buf.cell((x, y)) {
                    out.push_str(c.symbol());
                }
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn popup_shows_question_options_and_hint() {
        let text = rendered_text(false);
        assert!(text.contains("Question"), "title present");
        assert!(text.contains("Which database"), "question present");
        assert!(text.contains("sqlite"), "option 1 present");
        assert!(text.contains("postgres"), "option 2 present");
        assert!(text.contains("custom answer"), "custom row present");
        assert!(text.contains("Esc skip"), "hint present");
    }

    #[test]
    fn popup_respects_composer_top_anchor() {
        // composer_top = 20 in a 24-row terminal: the popup must live above it.
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let m = menu();
        terminal
            .draw(|f| {
                let area = f.area();
                render_question_popup(f, area, 20, &m);
            })
            .unwrap();
        // The title row must be strictly above the composer line.
        let buf = terminal.backend().buffer();
        let title_row = (0..20).find(|&y| {
            (0..80).any(|x| {
                buf.cell((x, y)).map(|c| c.symbol() == "Q").unwrap_or(false)
            })
        });
        assert!(title_row.is_some(), "title rendered above the composer");
    }
}
