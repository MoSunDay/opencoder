//! Rendering for the multi-question plan dialog.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

use super::state::{QuestionFocus, QuestionMenu};
use crate::theme;

const INPUT_PREFIX: &str = "✎ ";
// Ratatui/unicode-width renders the text-style pen as one cell. Keep this
// explicit instead of composer::char_width (which conservatively treats the
// whole dingbats range as emoji-wide).
const INPUT_PREFIX_WIDTH: u16 = 2;

/// Render the dialog above the composer and place the terminal cursor at the
/// active question's exact custom-input character position.
pub fn render_question_popup(
    frame: &mut Frame,
    area: Rect,
    composer_top: u16,
    menu: &QuestionMenu,
) {
    let item = menu.current();
    let inner_w = 56usize;
    let question_lines = wrapped_lines(&item.prompt.question, inner_w).max(1) as u16;
    let answer_rows = item.rows() as u16;
    // border + question + blank + answers + blank + input + blank + hint
    let wanted_height = 2 + question_lines + 1 + answer_rows + 1 + 1 + 1 + 1;
    let height = wanted_height.min(composer_top.max(1));
    let width = 60u16.min(area.width.saturating_sub(4));
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = composer_top.saturating_sub(height);
    let popup = Rect::new(x, y, width, height);
    frame.render_widget(Clear, popup);

    let mut lines = vec![Line::styled(
        item.prompt.question.clone(),
        Style::default()
            .fg(theme::text())
            .add_modifier(Modifier::BOLD),
    )];
    lines.push(Line::from(""));
    for (index, option) in item.prompt.options.iter().enumerate() {
        lines.push(option_line(
            option,
            menu.focus == QuestionFocus::Options && item.selected == index,
        ));
    }
    lines.push(option_line(
        "Custom",
        menu.focus == QuestionFocus::Options && item.selected == item.custom_row(),
    ));
    lines.push(Line::from(""));

    let input_width = popup
        .width
        .saturating_sub(2)
        .saturating_sub(INPUT_PREFIX_WIDTH)
        .saturating_sub(1) as usize;
    let focused = menu.focus == QuestionFocus::Custom;
    let (visible_input, cursor_column) = if focused {
        input_window(&item.custom_input, item.custom_cursor, input_width)
    } else if item.custom_input.is_empty() {
        ("add optional details…".to_string(), 0)
    } else {
        input_window(&item.custom_input, item.custom_cursor, input_width)
    };
    lines.push(input_line(&visible_input, focused));
    lines.push(Line::from(""));
    lines.push(Line::styled(
        "←→ question · ↑↓ answer · Tab input · Enter confirm · Esc skip",
        Style::default().fg(theme::muted()),
    ));

    let confirmation = if item.confirmed() { " ✓" } else { "" };
    let title = format!(
        " Question {}/{}{} ",
        menu.active + 1,
        menu.len(),
        confirmation
    );
    let block = crate::theme::rounded_block_plain().title(Span::styled(
        title,
        Style::default()
            .fg(theme::warn_color())
            .add_modifier(Modifier::BOLD),
    ));
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(ratatui::widgets::Wrap { trim: false }),
        popup,
    );

    if focused {
        let input_y = popup.y + 1 + question_lines + 1 + answer_rows + 1;
        let input_x = popup.x + 1 + INPUT_PREFIX_WIDTH + cursor_column as u16;
        if input_x < popup.x + popup.width.saturating_sub(1)
            && input_y < popup.y + popup.height.saturating_sub(1)
        {
            frame.set_cursor_position((input_x, input_y));
        }
    }
}

fn option_line(option: &str, selected: bool) -> Line<'static> {
    let marker = if selected { "▸ " } else { "  " };
    let style = if selected {
        Style::default()
            .fg(theme::accent())
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::text())
    };
    Line::from(Span::styled(format!("{marker}{option}"), style))
}

fn input_line(input: &str, focused: bool) -> Line<'static> {
    let style = if focused {
        Style::default()
            .fg(theme::warn_color())
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::muted())
    };
    Line::from(Span::styled(format!("{INPUT_PREFIX}{input}"), style))
}

/// Keep the cursor visible in a single terminal row. The returned column is
/// relative to the first rendered input character and uses display width,
/// not byte or char count.
fn input_window(input: &str, cursor: usize, max_width: usize) -> (String, usize) {
    let chars: Vec<char> = input.chars().collect();
    let cursor = cursor.min(chars.len());
    let mut start = cursor;
    let mut before_width = 0;
    while start > 0 {
        let width = crate::composer::char_width(chars[start - 1]);
        if before_width + width > max_width {
            break;
        }
        start -= 1;
        before_width += width;
    }

    let mut output: String = chars[start..cursor].iter().collect();
    let mut used = before_width;
    for ch in &chars[cursor..] {
        let width = crate::composer::char_width(*ch);
        if used + width > max_width {
            break;
        }
        output.push(*ch);
        used += width;
    }
    (output, before_width)
}

fn wrapped_lines(text: &str, width: usize) -> usize {
    let mut lines = 1;
    let mut column = 0;
    for word in text.split_whitespace() {
        let word_width = crate::composer::str_width(word) + 1;
        if column + word_width > width && column > 0 {
            lines += 1;
            column = word_width;
        } else {
            column += word_width;
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::question_menu::state::{handle_question_key, QuestionPrompt};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::backend::TestBackend;

    fn menu() -> QuestionMenu {
        let mut menu = QuestionMenu::new(QuestionPrompt {
            id: "q1".into(),
            question: "Which database engine should the migration target?".into(),
            options: vec!["sqlite".into(), "postgres".into()],
        });
        menu.push(QuestionPrompt {
            id: "q2".into(),
            question: "Which runtime?".into(),
            options: vec!["native".into()],
        });
        menu
    }

    fn rendered_text(menu: &QuestionMenu) -> String {
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_question_popup(frame, frame.area(), 20, menu))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let mut output = String::new();
        for y in 0..24 {
            for x in 0..80 {
                if let Some(cell) = buffer.cell((x, y)) {
                    output.push_str(cell.symbol());
                }
            }
            output.push('\n');
        }
        output
    }

    #[test]
    fn popup_shows_navigation_custom_option_and_separate_input() {
        let text = rendered_text(&menu());
        assert!(text.contains("Question 1/2"));
        assert!(text.contains("sqlite"));
        assert!(text.contains("Custom"));
        assert!(text.contains("✎ add optional details…"));
        assert!(text.contains("←→ question"));
    }

    #[test]
    fn cursor_starts_after_the_input_prefix() {
        let mut menu = menu();
        handle_question_key(&mut menu, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_question_popup(frame, frame.area(), 20, &menu))
            .unwrap();
        // popup x=10, content x=11, "✎ " occupies two columns.
        terminal.backend_mut().assert_cursor_position((13, 16));
    }

    #[test]
    fn cursor_uses_unicode_display_width_at_an_interior_position() {
        let mut menu = menu();
        handle_question_key(&mut menu, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        menu.paste_custom("你好a");
        handle_question_key(&mut menu, KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_question_popup(frame, frame.area(), 20, &menu))
            .unwrap();
        // Two CJK chars before the cursor occupy four columns.
        terminal.backend_mut().assert_cursor_position((17, 16));
    }

    #[test]
    fn long_input_window_keeps_cursor_inside_the_popup() {
        let input = "0123456789".repeat(8);
        let (visible, cursor) = input_window(&input, input.chars().count(), 52);
        assert_eq!(crate::composer::str_width(&visible), 52);
        assert_eq!(cursor, 52);
    }

    #[test]
    fn popup_stays_above_the_composer_anchor() {
        let text = rendered_text(&menu());
        let title_row = text.lines().position(|line| line.contains("Question 1/2"));
        assert!(title_row.is_some_and(|row| row < 20));
    }
}
