//! Rendering for the multi-question plan dialog.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

use super::state::{QuestionFocus, QuestionMenu};
use crate::theme;

const INPUT_HEIGHT: u16 = 3;
const HINT_HEIGHT: u16 = 1;
const PLACEHOLDER: &str = "add optional details…";

/// Render the dialog above the composer and place the terminal cursor at the
/// active question's exact custom-input character position.
pub fn render_question_popup(
    frame: &mut Frame,
    area: Rect,
    composer_top: u16,
    menu: &QuestionMenu,
) {
    let item = menu.current();
    let width = 60u16.min(area.width.saturating_sub(4));
    let inner_width = width.saturating_sub(2).max(1);
    let content_height = content_height(menu, inner_width);
    // Outer border + content + dedicated input box + hint. The popup is
    // capped at the composer anchor; the input remains pinned to its bottom.
    let wanted_height = 2 + content_height + INPUT_HEIGHT + HINT_HEIGHT;
    let height = wanted_height.min(composer_top.max(1));
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = composer_top.saturating_sub(height);
    let popup = Rect::new(x, y, width, height);
    frame.render_widget(Clear, popup);

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
    frame.render_widget(block, popup);

    let Some((content_area, input_area, hint_area)) = popup_sections(popup) else {
        return;
    };
    frame.render_widget(
        Paragraph::new(content_lines(menu)).wrap(ratatui::widgets::Wrap { trim: false }),
        content_area,
    );
    render_input(frame, input_area, menu);
    frame.render_widget(
        Paragraph::new("←→ question · ↑↓ answer · Tab input · Enter confirm · Esc skip")
            .style(Style::default().fg(theme::muted())),
        hint_area,
    );
}

/// Keep fixed controls independent from wrapped question/option content.
/// Returns `None` only when the terminal is too short to contain a real
/// three-row input box inside the popup border.
fn popup_sections(popup: Rect) -> Option<(Rect, Rect, Rect)> {
    let inner = Rect::new(
        popup.x.saturating_add(1),
        popup.y.saturating_add(1),
        popup.width.saturating_sub(2),
        popup.height.saturating_sub(2),
    );
    if inner.height < INPUT_HEIGHT + HINT_HEIGHT {
        return None;
    }
    let input_y = inner.bottom().saturating_sub(INPUT_HEIGHT + HINT_HEIGHT);
    let content = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        input_y.saturating_sub(inner.y),
    );
    let input = Rect::new(inner.x, input_y, inner.width, INPUT_HEIGHT);
    let hint = Rect::new(inner.x, input.bottom(), inner.width, HINT_HEIGHT);
    Some((content, input, hint))
}

fn content_lines(menu: &QuestionMenu) -> Vec<Line<'static>> {
    let item = menu.current();
    let mut lines = vec![Line::styled(
        item.prompt.question.clone(),
        Style::default()
            .fg(theme::text())
            .add_modifier(Modifier::BOLD),
    )];
    lines.push(Line::from(""));
    for (index, option) in item.prompt.options.iter().enumerate() {
        lines.push(option_line(
            &format!("{}. {option}", index + 1),
            menu.focus == QuestionFocus::Options && item.selected == index,
        ));
    }
    lines.push(option_line(
        "Custom",
        menu.focus == QuestionFocus::Options && item.selected == item.custom_row(),
    ));
    lines
}

fn option_line(display_value: &str, selected: bool) -> Line<'static> {
    let marker = if selected { "▸ " } else { "  " };
    let style = if selected {
        Style::default()
            .fg(theme::accent())
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::text())
    };
    Line::from(Span::styled(format!("{marker}{display_value}"), style))
}

fn render_input(frame: &mut Frame, area: Rect, menu: &QuestionMenu) {
    let item = menu.current();
    let focused = menu.focus == QuestionFocus::Custom;
    let block_style = if focused {
        Style::default().fg(theme::warn_color())
    } else {
        Style::default().fg(theme::muted())
    };
    let block = crate::theme::rounded_block_plain()
        .title(Span::styled(" Input ", block_style))
        .border_style(block_style);
    let inner = Rect::new(
        area.x.saturating_add(1),
        area.y.saturating_add(1),
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    );
    // Reserve the final cell for the hardware cursor so it never lands on
    // the right border when the visible window is full.
    let input_width = inner.width.saturating_sub(1) as usize;
    let (visible_input, cursor_column) =
        input_window(&item.custom_input, item.custom_cursor, input_width);
    let display = if !focused && item.custom_input.is_empty() {
        PLACEHOLDER.to_string()
    } else {
        visible_input
    };
    let style = if focused {
        Style::default()
            .fg(theme::warn_color())
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::muted())
    };
    frame.render_widget(Paragraph::new(display).style(style).block(block), area);

    if focused && inner.width > 0 {
        let cursor_x = inner.x + (cursor_column as u16).min(inner.width.saturating_sub(1));
        frame.set_cursor_position((cursor_x, inner.y));
    }
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

fn content_height(menu: &QuestionMenu, width: u16) -> u16 {
    Paragraph::new(content_lines(menu))
        .wrap(ratatui::widgets::Wrap { trim: false })
        .line_count(width)
        .min(u16::MAX as usize) as u16
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
        assert!(text.contains("1. sqlite"));
        assert!(text.contains("2. postgres"));
        assert!(text.contains("Custom"));
        assert!(!text.contains("3. Custom"));
        assert!(text.contains("Input"));
        assert!(text.contains(PLACEHOLDER));
        assert!(text.contains("←→ question"));
    }

    #[test]
    fn display_numbers_do_not_change_the_answer_value() {
        let mut menu = menu();
        handle_question_key(&mut menu, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(menu.current().current_answer().as_deref(), Some("postgres"));
        let text = rendered_text(&menu);
        assert!(text.contains("2. postgres"));
    }

    #[test]
    fn cursor_starts_inside_the_input_box() {
        let mut menu = menu();
        handle_question_key(&mut menu, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_question_popup(frame, frame.area(), 20, &menu))
            .unwrap();
        // popup x=10; nested input content starts at x=12, row 16.
        terminal.backend_mut().assert_cursor_position((12, 16));
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
        terminal.backend_mut().assert_cursor_position((16, 16));
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

    #[test]
    fn wrapped_content_cannot_move_or_hide_the_input_cursor() {
        let mut menu = QuestionMenu::new(QuestionPrompt {
            id: "q1".into(),
            question: "这是一个会在窄终端中换行很多次的长问题".repeat(3),
            options: vec!["a very long option value that must wrap repeatedly".repeat(2)],
        });
        handle_question_key(&mut menu, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        menu.paste_custom("你好a");
        handle_question_key(&mut menu, KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));

        let backend = TestBackend::new(34, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_question_popup(frame, frame.area(), 16, &menu))
            .unwrap();

        // Fixed input geometry: popup bottom is row 16, input content row is
        // always 12. Two CJK characters occupy four columns from x=4.
        terminal.backend_mut().assert_cursor_position((8, 12));
    }
}
