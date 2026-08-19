//! Unit tests for the question-dialog state machine.

use super::*;

fn prompt(id: &str, question: &str) -> QuestionPrompt {
    QuestionPrompt {
        id: id.into(),
        question: question.into(),
        options: vec!["sqlite".into(), "postgres".into()],
    }
}

fn menu() -> QuestionMenu {
    let mut menu = QuestionMenu::new(prompt("q1", "Database?"));
    menu.push(prompt("q2", "Runtime?"));
    menu
}

/// Terminal-wide (80 col) question popup wrap width.
const WIDTH: u16 = 55;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn press(menu: &mut QuestionMenu, code: KeyCode) -> QuestionAction {
    handle_question_key(menu, key(code), WIDTH)
}

fn press_with(menu: &mut QuestionMenu, event: KeyEvent, width: u16) -> QuestionAction {
    handle_question_key(menu, event, width)
}

fn ctrl(ch: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)
}

fn alt(ch: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(ch), KeyModifiers::ALT)
}

#[test]
fn arrows_switch_questions_and_preserve_each_questions_state() {
    let mut menu = menu();
    press(&mut menu, KeyCode::Down);
    press(&mut menu, KeyCode::Tab);
    for ch in "first".chars() {
        press(&mut menu, KeyCode::Char(ch));
    }
    press(&mut menu, KeyCode::Tab);
    press(&mut menu, KeyCode::Right);
    press(&mut menu, KeyCode::Up);
    press(&mut menu, KeyCode::Tab);
    for ch in "second".chars() {
        press(&mut menu, KeyCode::Char(ch));
    }
    press(&mut menu, KeyCode::Tab);
    press(&mut menu, KeyCode::Left);

    assert_eq!(menu.active, 0);
    assert_eq!(menu.current().selected, 1);
    assert_eq!(menu.current().custom_input, "first");
    assert_eq!(menu.current().custom_cursor, 5);
    assert_eq!(menu.questions[1].selected, 2);
    assert_eq!(menu.questions[1].custom_input, "second");
}

#[test]
fn preset_answer_appends_custom_input_but_custom_option_uses_only_input() {
    let mut menu = menu();
    press(&mut menu, KeyCode::Down);
    press(&mut menu, KeyCode::Tab);
    menu.paste_custom("version 16");
    assert_eq!(
        menu.current().current_answer().as_deref(),
        Some("postgres\nversion 16")
    );

    press(&mut menu, KeyCode::Tab);
    press(&mut menu, KeyCode::Down);
    assert_eq!(menu.current().selected, menu.current().custom_row());
    assert_eq!(
        menu.current().current_answer().as_deref(),
        Some("version 16")
    );
}

#[test]
fn confirmations_are_held_until_every_question_is_confirmed() {
    let mut menu = menu();
    assert_eq!(press(&mut menu, KeyCode::Enter), QuestionAction::Idle);
    assert_eq!(menu.active, 1);
    assert!(menu.questions[0].confirmed());
    assert!(!menu.questions[1].confirmed());

    assert_eq!(
        press(&mut menu, KeyCode::Enter),
        QuestionAction::Submit(vec![
            QuestionResponse {
                id: "q1".into(),
                answer: Some("sqlite".into())
            },
            QuestionResponse {
                id: "q2".into(),
                answer: Some("sqlite".into())
            },
        ])
    );
}

#[test]
fn editing_a_confirmed_question_requires_reconfirmation() {
    let mut menu = menu();
    press(&mut menu, KeyCode::Enter);
    press(&mut menu, KeyCode::Left);
    press(&mut menu, KeyCode::Down);
    assert!(!menu.questions[0].confirmed());
    press(&mut menu, KeyCode::Right);
    assert_eq!(press(&mut menu, KeyCode::Enter), QuestionAction::Idle);
    assert_eq!(menu.active, 0);
}

#[test]
fn custom_option_requires_text_and_enter_focuses_the_input() {
    let mut menu = QuestionMenu::new(QuestionPrompt {
        id: "q1".into(),
        question: "Free form?".into(),
        options: vec![],
    });
    assert_eq!(press(&mut menu, KeyCode::Enter), QuestionAction::Idle);
    assert_eq!(menu.focus, QuestionFocus::Custom);
    assert_eq!(press(&mut menu, KeyCode::Enter), QuestionAction::Idle);
    press(&mut menu, KeyCode::Char('中'));
    assert_eq!(
        press(&mut menu, KeyCode::Enter),
        QuestionAction::Submit(vec![QuestionResponse {
            id: "q1".into(),
            answer: Some("中".into()),
        }])
    );
}

#[test]
fn custom_cursor_edits_unicode_by_character_index() {
    let mut menu = menu();
    press(&mut menu, KeyCode::Tab);
    menu.paste_custom("你好a");
    press(&mut menu, KeyCode::Left);
    press(&mut menu, KeyCode::Left);
    press(&mut menu, KeyCode::Char('X'));
    assert_eq!(menu.current().custom_input, "你X好a");
    press(&mut menu, KeyCode::Backspace);
    assert_eq!(menu.current().custom_input, "你好a");
    assert_eq!(menu.current().custom_cursor, 1);
}

#[test]
fn paste_preserves_newlines_and_aligns_the_cursor() {
    let mut menu = menu();
    menu.paste_custom("first\nsecond\tpart");
    assert_eq!(menu.current().custom_input, "first\nsecond    part");
    assert_eq!(
        menu.current().custom_cursor,
        menu.current().custom_input.chars().count()
    );
}

#[test]
fn readline_jump_keys_reach_line_boundaries() {
    let mut menu = menu();
    press(&mut menu, KeyCode::Tab);
    menu.paste_custom("one two\nthree");
    // Jump keys operate on the logical line, not the whole buffer.
    press_with(&mut menu, ctrl('a'), WIDTH);
    assert_eq!(menu.current().custom_cursor, 8); // start of "three"
    press(&mut menu, KeyCode::Home);
    assert_eq!(menu.current().custom_cursor, 8);
    press_with(&mut menu, ctrl('e'), WIDTH);
    assert_eq!(menu.current().custom_cursor, 13);
    press(&mut menu, KeyCode::End);
    assert_eq!(menu.current().custom_cursor, 13);
    for _ in 0..6 {
        press(&mut menu, KeyCode::Left); // onto the first line
    }
    press(&mut menu, KeyCode::Home);
    assert_eq!(menu.current().custom_cursor, 0);
    press_with(&mut menu, ctrl('e'), WIDTH);
    assert_eq!(menu.current().custom_cursor, 7);
}

#[test]
fn ctrl_u_clears_and_ctrl_k_deletes_to_the_end() {
    let mut menu = menu();
    press(&mut menu, KeyCode::Tab);
    menu.paste_custom("hello world");
    for _ in 0..6 {
        press(&mut menu, KeyCode::Left);
    }
    assert_eq!(menu.current().custom_cursor, 5);
    press_with(&mut menu, ctrl('k'), WIDTH);
    assert_eq!(menu.current().custom_input, "hello");
    assert_eq!(menu.current().custom_cursor, 5);
    press_with(&mut menu, ctrl('u'), WIDTH);
    assert_eq!(menu.current().custom_input, "");
    assert_eq!(menu.current().custom_cursor, 0);
}

#[test]
fn word_keys_delete_and_move_by_word() {
    let mut menu = menu();
    press(&mut menu, KeyCode::Tab);
    menu.paste_custom("foo bar baz");
    for _ in 0..3 {
        press(&mut menu, KeyCode::Left);
    }
    assert_eq!(menu.current().custom_cursor, 8);
    press_with(&mut menu, ctrl('w'), WIDTH);
    assert_eq!(menu.current().custom_input, "foo baz");
    assert_eq!(menu.current().custom_cursor, 4);
    press_with(&mut menu, alt('f'), WIDTH);
    assert_eq!(menu.current().custom_cursor, 7); // end of "baz"
    press_with(&mut menu, alt('b'), WIDTH);
    assert_eq!(menu.current().custom_cursor, 4);
    press_with(
        &mut menu,
        KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT),
        WIDTH,
    );
    assert_eq!(menu.current().custom_input, "baz");
    assert_eq!(menu.current().custom_cursor, 0);
}

#[test]
fn delete_key_removes_the_char_under_the_cursor() {
    let mut menu = menu();
    press(&mut menu, KeyCode::Tab);
    menu.paste_custom("abc");
    press(&mut menu, KeyCode::Home);
    press(&mut menu, KeyCode::Delete);
    assert_eq!(menu.current().custom_input, "bc");
    assert_eq!(menu.current().custom_cursor, 0);
    press(&mut menu, KeyCode::Delete);
    press(&mut menu, KeyCode::Delete);
    assert_eq!(menu.current().custom_input, "");
}

#[test]
fn explicit_newline_keys_keep_enter_as_confirm() {
    let mut menu = QuestionMenu::new(prompt("q1", "Free form?"));
    press(&mut menu, KeyCode::Tab);
    menu.paste_custom("line1");
    press_with(
        &mut menu,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT),
        WIDTH,
    );
    press_with(&mut menu, ctrl('j'), WIDTH);
    press(&mut menu, KeyCode::Char('x'));
    assert_eq!(menu.current().custom_input, "line1\n\nx");
    assert_eq!(
        press(&mut menu, KeyCode::Enter),
        QuestionAction::Submit(vec![QuestionResponse {
            id: "q1".into(),
            // Preset option selected: custom details append on a new line.
            answer: Some("sqlite\nline1\n\nx".into()),
        }])
    );
}

#[test]
fn up_down_move_across_wrapped_rows_before_leaving_the_input() {
    let mut menu = menu();
    press_with(&mut menu, key(KeyCode::Tab), WIDTH);
    menu.paste_custom("aaaaaaaaaaaa"); // wraps to 2 rows at width 10
    assert_eq!(menu.focus, QuestionFocus::Custom);
    press_with(&mut menu, key(KeyCode::Up), 10);
    assert_eq!(menu.current().custom_cursor, 2);
    assert_eq!(menu.focus, QuestionFocus::Custom);
    press_with(&mut menu, key(KeyCode::Up), 10);
    assert_eq!(menu.focus, QuestionFocus::Options);
    press_with(&mut menu, key(KeyCode::Tab), WIDTH);
    press_with(&mut menu, key(KeyCode::Down), 10);
    assert_eq!(menu.current().custom_cursor, 12);
    // Down on the last visual row is a no-op that keeps the focus.
    press_with(&mut menu, key(KeyCode::Down), 10);
    assert_eq!(menu.current().custom_cursor, 12);
    assert_eq!(menu.focus, QuestionFocus::Custom);
}

#[test]
fn up_crosses_explicit_newlines_too() {
    let mut menu = menu();
    press(&mut menu, KeyCode::Tab);
    menu.paste_custom("one\ntwo");
    press(&mut menu, KeyCode::Up);
    assert_eq!(menu.current().custom_cursor, 3);
    assert_eq!(menu.focus, QuestionFocus::Custom);
    press(&mut menu, KeyCode::Up);
    assert_eq!(menu.focus, QuestionFocus::Options);
}

#[test]
fn skip_is_batched_with_answers() {
    let mut menu = menu();
    assert_eq!(press(&mut menu, KeyCode::Esc), QuestionAction::Idle);
    assert_eq!(
        press(&mut menu, KeyCode::Enter),
        QuestionAction::Submit(vec![
            QuestionResponse {
                id: "q1".into(),
                answer: None
            },
            QuestionResponse {
                id: "q2".into(),
                answer: Some("sqlite".into())
            },
        ])
    );
}
