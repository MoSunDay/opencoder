//! Rendering for the multi-question plan dialog.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

use super::state::{QuestionFocus, QuestionMenu};
use crate::{composer, theme};

/// Soft-wrapped custom-input rows before the box starts scrolling inside.
const MAX_INPUT_ROWS: u16 = 6;
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
    let wrap_w = super::input_wrap_width(area.width);
    // The input box grows with its soft-wrapped rows (capped) so long
    // answers stay visible instead of scrolling inside a single row.
    let input_height = input_box_height(&item.custom_input, wrap_w);
    let content_height = content_height(menu, inner_width);
    // Outer border + content + dedicated input box + hint. The popup is
    // capped at the composer anchor; the input remains pinned to its bottom.
    let wanted_height = 2 + content_height + input_height + HINT_HEIGHT;
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

    let Some((content_area, input_area, hint_area)) = popup_sections(popup, input_height) else {
        return;
    };
    frame.render_widget(
        Paragraph::new(content_lines(menu)).wrap(ratatui::widgets::Wrap { trim: false }),
        content_area,
    );
    render_input(frame, input_area, menu, wrap_w);
    frame.render_widget(
        Paragraph::new("←→ question · ↑↓ answer · Tab input · Enter confirm · Esc skip")
            .style(Style::default().fg(theme::muted())),
        hint_area,
    );
}

/// Keep fixed controls independent from wrapped question/option content.
/// Returns `None` only when the terminal is too short to contain the input
/// box (plus hint) inside the popup border.
fn popup_sections(popup: Rect, input_height: u16) -> Option<(Rect, Rect, Rect)> {
    let inner = Rect::new(
        popup.x.saturating_add(1),
        popup.y.saturating_add(1),
        popup.width.saturating_sub(2),
        popup.height.saturating_sub(2),
    );
    if inner.height < input_height + HINT_HEIGHT {
        return None;
    }
    let input_y = inner.bottom().saturating_sub(input_height + HINT_HEIGHT);
    let content = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        input_y.saturating_sub(inner.y),
    );
    let input = Rect::new(inner.x, input_y, inner.width, input_height);
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
    // Selection stays highlighted regardless of focus: the input box border
    // (warn vs muted) already shows where keyboard input currently goes.
    for (index, option) in item.prompt.options.iter().enumerate() {
        lines.push(option_line(
            &format!("{}. {option}", index + 1),
            item.selected == index,
        ));
    }
    lines.push(option_line("Custom", item.selected == item.custom_row()));
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

/// Input box height: soft-wrapped rows (capped) plus the two border rows.
fn input_box_height(input: &str, wrap_w: u16) -> u16 {
    composer::display_rows(input, wrap_w, 0).min(MAX_INPUT_ROWS) + 2
}

/// Soft-wrap the custom input and vertically scroll it so the cursor row
/// stays inside the box. Returns the visible rows plus the cursor's row
/// inside that window.
fn scrolled_input_rows(
    input: &str,
    cursor: usize,
    wrap_w: u16,
    visible_h: u16,
) -> (Vec<Line<'static>>, usize) {
    let rows = composer::wrap_rows(input, wrap_w, 0);
    let (cursor_row, _) = composer::cursor_row_col(input, cursor, wrap_w, 0);
    let visible = visible_h.max(1) as usize;
    let scroll = cursor_row
        .saturating_sub(visible - 1)
        .min(rows.len().saturating_sub(1));
    let chars: Vec<char> = input.chars().collect();
    let lines = rows
        .iter()
        .skip(scroll)
        .take(visible)
        .map(|row| Line::from(chars[row.start..row.end].iter().collect::<String>()))
        .collect();
    (lines, cursor_row.saturating_sub(scroll))
}

fn render_input(frame: &mut Frame, area: Rect, menu: &QuestionMenu, wrap_w: u16) {
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
    let (mut rows, cursor_window_row) =
        scrolled_input_rows(&item.custom_input, item.custom_cursor, wrap_w, inner.height);
    if item.custom_input.is_empty() && !focused {
        rows = vec![Line::from(PLACEHOLDER)];
    }
    let style = if focused {
        Style::default()
            .fg(theme::warn_color())
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::muted())
    };
    frame.render_widget(Paragraph::new(rows).style(style).block(block), area);

    if focused && inner.width > 0 && inner.height > 0 {
        let (_, cursor_column) =
            composer::cursor_row_col(&item.custom_input, item.custom_cursor, wrap_w, 0);
        // `wrap_w` already reserves the final cell so the cursor can never
        // land on the right border of a full row.
        let cursor_x = inner.x + (cursor_column as u16).min(inner.width.saturating_sub(1));
        let cursor_y = inner.y + cursor_window_row as u16;
        frame.set_cursor_position((cursor_x, cursor_y));
    }
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

    /// 80-column terminal: `input_wrap_width(80)` == 55.
    const WIDTH: u16 = 55;

    fn press(menu: &mut QuestionMenu, code: KeyCode) {
        handle_question_key(menu, KeyEvent::new(code, KeyModifiers::NONE), WIDTH);
    }

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
        press(&mut menu, KeyCode::Down);
        assert_eq!(menu.current().current_answer().as_deref(), Some("postgres"));
        let text = rendered_text(&menu);
        assert!(text.contains("2. postgres"));
    }

    #[test]
    fn cursor_starts_inside_the_input_box() {
        let mut menu = menu();
        press(&mut menu, KeyCode::Tab);
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
        press(&mut menu, KeyCode::Tab);
        menu.paste_custom("你好a");
        press(&mut menu, KeyCode::Left);
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_question_popup(frame, frame.area(), 20, &menu))
            .unwrap();
        // Two CJK chars before the cursor occupy four columns.
        terminal.backend_mut().assert_cursor_position((16, 16));
    }

    #[test]
    fn selected_row_stays_highlighted_while_the_input_has_focus() {
        let mut menu = menu();
        press(&mut menu, KeyCode::Down); // postgres selected
        press(&mut menu, KeyCode::Tab); // focus the custom input
        assert_eq!(menu.focus, QuestionFocus::Custom);
        let text = rendered_text(&menu);
        let selected = text
            .lines()
            .find(|line| line.contains("2. postgres"))
            .expect("postgres row rendered");
        assert!(selected.contains("▸ 2. postgres"));
        let unselected = text
            .lines()
            .find(|line| line.contains("1. sqlite"))
            .expect("sqlite row rendered");
        assert!(!unselected.contains("▸ 1. sqlite"));
        let custom = text
            .lines()
            .find(|line| line.contains("Custom"))
            .expect("custom row rendered");
        assert!(!custom.contains("▸"));
    }

    #[test]
    fn wrapped_input_grows_the_input_box_and_tracks_the_cursor() {
        let mut menu = menu();
        press(&mut menu, KeyCode::Tab);
        menu.paste_custom(&"0123456789".repeat(12)); // 120 chars -> 3 rows of 55
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_question_popup(frame, frame.area(), 20, &menu))
            .unwrap();
        // Popup bottom pinned at row 19: hint 19, input box 13..17, content
        // rows 14..16 with the cursor on the last one, 10 columns into the
        // wrapped third row (x = 12 + 10).
        terminal.backend_mut().assert_cursor_position((22, 16));
        let buffer = terminal.backend().buffer();
        let row = (0..80)
            .map(|x| buffer.cell((x, 16)).map(|c| c.symbol()).unwrap_or(""))
            .collect::<String>();
        assert!(row.contains("0123456789"), "third wrapped row visible");
    }

    #[test]
    fn input_taller_than_the_cap_scrolls_to_keep_the_cursor_visible() {
        let mut menu = menu();
        press(&mut menu, KeyCode::Tab);
        // Eight distinct 54-char rows: the box caps at 6 and scrolls so the
        // cursor (end of row 8) stays visible while row 1 scrolls out.
        let text = (1..=8)
            .map(|i| format!("row{i}{}", "x".repeat(50)))
            .collect::<Vec<_>>()
            .join("\n");
        menu.paste_custom(&text);
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_question_popup(frame, frame.area(), 20, &menu))
            .unwrap();
        // Box rows 11..16 (6 content rows): rows 3..8 visible, cursor at the
        // tail of row 8 (x = 12 + 54).
        terminal.backend_mut().assert_cursor_position((66, 16));
        let buffer = terminal.backend().buffer();
        let top = (0..80)
            .map(|x| buffer.cell((x, 11)).map(|c| c.symbol()).unwrap_or(""))
            .collect::<String>();
        assert!(top.contains("row3"), "first visible row is row3: {top:?}");
        assert!(!top.contains("row1"), "row1 scrolled out: {top:?}");
    }

    #[test]
    fn explicit_newlines_also_grow_the_input_box() {
        let mut menu = menu();
        press(&mut menu, KeyCode::Tab);
        menu.paste_custom("one\ntwo\nthree");
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_question_popup(frame, frame.area(), 20, &menu))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text_of = |y: u16| {
            (0..80)
                .map(|x| buffer.cell((x, y)).map(|c| c.symbol()).unwrap_or(""))
                .collect::<String>()
        };
        // 3 content rows: rows 14..16 inside the box; "two" on the middle one.
        assert!(text_of(15).contains("two"));
        terminal.backend_mut().assert_cursor_position((17, 16));
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
        press(&mut menu, KeyCode::Tab);
        menu.paste_custom("你好a");
        press(&mut menu, KeyCode::Left);

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
