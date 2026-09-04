use super::*;

#[test]
fn resume_hint_is_copyable_command() {
    assert_eq!(resume_hint("01ABC"), "resume with: opencoder -s 01ABC");
}

#[test]
fn enter_submits_non_empty_input() {
    let mut input = String::from("hello world");
    let mut idx = 11;
    let action = run_handle(
        key(KeyCode::Enter, KeyModifiers::NONE),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(matches!(action, KeyAction::Submit(ref t) if t == "hello world"));
    assert!(input.is_empty());
    assert_eq!(idx, 0);
}

#[test]
fn enter_empty_input_is_noop() {
    let mut input = String::new();
    let mut idx = 0;
    let action = run_handle(
        key(KeyCode::Enter, KeyModifiers::NONE),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(matches!(action, KeyAction::None));
}

#[test]
fn enter_while_running_admits_steer() {
    let mut input = String::from("stop and rethink");
    let mut idx = 15;
    let action = run_handle(
        key(KeyCode::Enter, KeyModifiers::NONE),
        &mut input,
        &mut idx,
        true,
        "act",
    );
    assert!(matches!(action, KeyAction::Steer(ref t) if t == "stop and rethink"));
    assert!(input.is_empty());
}

#[test]
fn enter_with_shift_inserts_newline() {
    let mut input = String::from("hello");
    let mut idx = 5;
    let action = run_handle(
        key(KeyCode::Enter, KeyModifiers::SHIFT),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(matches!(action, KeyAction::None));
    assert_eq!(input, "hello\n");
    assert_eq!(idx, 6);
}

#[test]
fn enter_with_alt_inserts_newline() {
    let mut input = String::from("hi");
    let mut idx = 2;
    let action = run_handle(
        key(KeyCode::Enter, KeyModifiers::ALT),
        &mut input,
        &mut idx,
        true,
        "act",
    );
    assert!(matches!(action, KeyAction::None));
    assert_eq!(input, "hi\n");
}

#[test]
fn ctrl_j_inserts_newline() {
    let mut input = String::from("ab");
    let mut idx = 2;
    let action = run_handle(
        key(KeyCode::Char('j'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(matches!(action, KeyAction::None));
    assert_eq!(input, "ab\n");
}

#[test]
fn ctrl_u_clears_entire_input_line() {
    let mut input = String::from("hello world");
    let mut idx = 11;
    let action = run_handle(
        key(KeyCode::Char('u'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(matches!(action, KeyAction::None));
    assert!(input.is_empty(), "Ctrl+U must clear the entire input line");
    assert_eq!(idx, 0, "Ctrl+U must reset the cursor to 0");
}

#[test]
fn ctrl_u_on_empty_input_is_noop() {
    let mut input = String::new();
    let mut idx = 0;
    let action = run_handle(
        key(KeyCode::Char('u'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(matches!(action, KeyAction::None));
    assert!(input.is_empty(), "Ctrl+U on empty input must be a no-op");
    assert_eq!(idx, 0, "Ctrl+U on empty input must not move the cursor");
}

#[test]
fn tab_while_running_admits_queue() {
    let mut input = String::from("next task");
    let mut idx = 9;
    let action = run_handle(
        key(KeyCode::Tab, KeyModifiers::NONE),
        &mut input,
        &mut idx,
        true,
        "act",
    );
    assert!(matches!(action, KeyAction::Queue(ref t) if t == "next task"));
}

#[test]
fn tab_while_idle_submits() {
    let mut input = String::from("hello");
    let mut idx = 5;
    let action = run_handle(
        key(KeyCode::Tab, KeyModifiers::NONE),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(matches!(action, KeyAction::Submit(ref t) if t == "hello"));
}

#[test]
fn tab_with_live_subagents_admits_queue() {
    // Idle parent + live subagents (autopilot stage gap, cancel grace,
    // reabsorb tail): Tab must queue, not submit — the queued item is
    // consumed by the same serial worker after the subagent batch ends.
    let mut input = String::from("follow-up");
    let mut idx = 9;
    let action = run_handle_subagents_busy(
        key(KeyCode::Tab, KeyModifiers::NONE),
        &mut input,
        &mut idx,
        "act",
    );
    assert!(matches!(action, KeyAction::Queue(ref t) if t == "follow-up"));
}

#[test]
fn tab_on_focused_subagent_rejected_not_queued() {
    // Focusing a *running* subagent enables input (Enter => steer) but Tab
    // must NOT queue: a queue is admitted to the parent session and would
    // affect the parent agent. Instead the input is preserved untouched.
    let mut input = String::from("keep this text");
    let mut idx = 14usize;
    let action = run_handle_subagent(
        key(KeyCode::Tab, KeyModifiers::NONE),
        &mut input,
        &mut idx,
        "act",
    );
    assert!(matches!(action, KeyAction::QueueUnsupported));
    assert_eq!(input, "keep this text");
    assert_eq!(idx, 14);
}

#[test]
fn enter_on_focused_subagent_steers_the_child() {
    // Enter on a focused running subagent steers the CHILD session — the
    // parent's steer panel, turn and queue are all untouched, and the input
    // line is cleared for the next child steer.
    let mut input = String::from("steer it");
    let mut idx = 8usize;
    let action = run_handle_subagent(
        key(KeyCode::Enter, KeyModifiers::NONE),
        &mut input,
        &mut idx,
        "act",
    );
    assert!(matches!(action, KeyAction::SubagentSteer(ref t) if t == "steer it"));
    assert!(input.is_empty());
    assert_eq!(idx, 0);
}

#[test]
fn tab_empty_input_on_focused_subagent_is_noop() {
    // Empty input + Tab on a focused subagent is a no-op (consistent with
    // the normal empty-input guard), not a QueueUnsupported.
    let mut input = String::new();
    let mut idx = 0usize;
    let action = run_handle_subagent(
        key(KeyCode::Tab, KeyModifiers::NONE),
        &mut input,
        &mut idx,
        "act",
    );
    assert!(matches!(action, KeyAction::None));
}

#[test]
fn ctrl_o_is_not_steer() {
    // Ctrl+O was removed as a steer trigger (replaced by Enter while running).
    // Verify it does NOT produce a Steer action.
    let mut input = String::from("msg");
    let mut idx = 3;
    let action = run_handle(
        key(KeyCode::Char('o'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        true,
        "act",
    );
    assert!(
        !matches!(action, KeyAction::Steer(_)),
        "Ctrl+O must not steer; got {action:?}"
    );
}

#[test]
fn ctrl_j_is_not_queue() {
    // Ctrl+J was removed as a queue trigger (replaced by Tab while running).
    // Verify it does NOT produce a Queue action.
    let mut input = String::from("msg");
    let mut idx = 3;
    let action = run_handle(
        key(KeyCode::Char('j'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        true,
        "act",
    );
    assert!(
        !matches!(action, KeyAction::Queue(_)),
        "Ctrl+J must not queue; got {action:?}"
    );
}

#[test]
fn left_right_move_cursor() {
    let mut input = String::from("abc");
    let mut idx = 3;
    run_handle(
        key(KeyCode::Left, KeyModifiers::NONE),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert_eq!(idx, 2);
    run_handle(
        key(KeyCode::Left, KeyModifiers::NONE),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert_eq!(idx, 1);
    run_handle(
        key(KeyCode::Right, KeyModifiers::NONE),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert_eq!(idx, 2);
}

#[test]
fn ctrl_a_moves_cursor_to_start() {
    let mut input = String::from("hello");
    let mut idx = 4;
    let action = run_handle(
        key(KeyCode::Char('a'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(matches!(action, KeyAction::None));
    assert_eq!(idx, 0, "Ctrl+A must move cursor to the first char");
    assert_eq!(input, "hello", "Ctrl+A must not mutate the input");
}

#[test]
fn ctrl_e_moves_cursor_to_end() {
    let mut input = String::from("hello");
    let mut idx = 1;
    let action = run_handle(
        key(KeyCode::Char('e'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(matches!(action, KeyAction::None));
    assert_eq!(idx, 5, "Ctrl+E must move cursor past the last char");
    assert_eq!(input, "hello", "Ctrl+E must not mutate the input");
}

#[test]
fn ctrl_a_e_on_empty_input_stay_at_zero() {
    let mut input = String::new();
    let mut idx = 0;
    run_handle(
        key(KeyCode::Char('a'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert_eq!(idx, 0);
    run_handle(
        key(KeyCode::Char('e'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert_eq!(idx, 0, "Ctrl+E on empty input must stay at 0");
}

#[test]
fn ctrl_a_e_handle_multibyte_chars() {
    // "héllo" is 5 chars but 6 bytes; cursor_idx is a char index.
    let mut input = String::from("héllo");
    let mut idx = 3;
    run_handle(
        key(KeyCode::Char('a'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert_eq!(idx, 0);
    run_handle(
        key(KeyCode::Char('e'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert_eq!(idx, 5, "Ctrl+E must land at char count, not byte length");
}

#[test]
fn ctrl_w_deletes_word_before_cursor() {
    let mut input = String::from("hello world");
    let mut idx = 11;
    let action = run_handle(
        key(KeyCode::Char('w'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(matches!(action, KeyAction::None));
    assert_eq!(input, "hello ");
    assert_eq!(idx, 6, "Ctrl+W must move cursor to end of remaining text");
}

#[test]
fn ctrl_w_at_start_is_noop() {
    let mut input = String::from("hello");
    let mut idx = 0;
    let action = run_handle(
        key(KeyCode::Char('w'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(matches!(action, KeyAction::None));
    assert_eq!(input, "hello", "Ctrl+W at start must not mutate input");
    assert_eq!(idx, 0);
}

#[test]
fn ctrl_w_empty_input_is_noop() {
    let mut input = String::new();
    let mut idx = 0;
    let action = run_handle(
        key(KeyCode::Char('w'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(matches!(action, KeyAction::None));
    assert!(input.is_empty());
    assert_eq!(idx, 0);
}

#[test]
fn ctrl_w_does_not_cross_newline() {
    let mut input = String::from("line1\nline2");
    let mut idx = 11;
    let action = run_handle(
        key(KeyCode::Char('w'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(matches!(action, KeyAction::None));
    assert_eq!(input, "line1\n", "Ctrl+W must not delete across newlines");
    assert_eq!(idx, 6);
}

#[test]
fn ctrl_w_trailing_whitespace() {
    // "hello   |" → "" — Ctrl+W deletes word + trailing whitespace (bash behavior)
    let mut input = String::from("hello   ");
    let mut idx = 8;
    let action = run_handle(
        key(KeyCode::Char('w'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(matches!(action, KeyAction::None));
    assert_eq!(input, "");
    assert_eq!(idx, 0);
}

#[test]
fn ctrl_w_multibyte_chars() {
    // "你好 world|" → "你好 |"
    let mut input = String::from("你好 world");
    let mut idx = 8;
    let action = run_handle(
        key(KeyCode::Char('w'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(matches!(action, KeyAction::None));
    assert_eq!(input, "你好 ");
    assert_eq!(idx, 3, "cursor must be at char boundary after 你好 ");
}

#[test]
fn alt_f_moves_cursor_forward_word() {
    let mut input = String::from("foo bar");
    let mut cursor = 0usize;
    let action = run_handle(
        key(KeyCode::Char('f'), KeyModifiers::ALT),
        &mut input,
        &mut cursor,
        false,
        "act",
    );
    assert!(matches!(action, KeyAction::None));
    assert_eq!(cursor, 3);
}

#[test]
fn alt_b_moves_cursor_backward_word() {
    let mut input = String::from("foo bar");
    let mut cursor = 7usize;
    let action = run_handle(
        key(KeyCode::Char('b'), KeyModifiers::ALT),
        &mut input,
        &mut cursor,
        false,
        "act",
    );
    assert!(matches!(action, KeyAction::None));
    assert_eq!(cursor, 4);
}
