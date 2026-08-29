use super::*;

fn press_running_command(
    command: &str,
    code: KeyCode,
    subagent_focused: bool,
) -> (KeyAction, String, usize) {
    let mut input = command.to_string();
    let mut cursor = input.chars().count();
    let mut hist_idx = None;
    let mut scroll = 0;
    let mut follow = true;
    let mut last_esc = None;
    let mut skill_menu = None;
    let mut undo_state = crate::undo::init(&input, cursor);
    let mut queue_scroll = 0;
    let mut file_menu = None;
    let action = handle_key(
        KeyEvent::new(code, KeyModifiers::NONE),
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
        &mut input,
        &mut cursor,
        &[],
        &mut hist_idx,
        true,
        "act",
        &mut scroll,
        &mut follow,
        &mut last_esc,
        &mut skill_menu,
        80,
        2,
        subagent_focused,
        false,
        &mut undo_state,
        &mut queue_scroll,
        &mut file_menu,
        Path::new("."),
    );
    (action, input, cursor)
}

fn press_running_mode_command(command: &str, code: KeyCode) -> (KeyAction, String, usize) {
    press_running_command(command, code, false)
}

/// Enter on a mode command while the parent runs admits it as a steer: the
/// runner applies it at the next turn boundary (delayed application).
#[test]
fn running_enter_mode_command_becomes_steer() {
    for command in [
        "/sandbox",
        "/act",
        "/sandbox review this",
        "/clear_context now",
    ] {
        let (action, input, _) = press_running_mode_command(command, KeyCode::Enter);
        assert!(matches!(action, KeyAction::Steer(text) if text == command));
        assert!(input.is_empty(), "steer clears the input line");
    }
}

/// Tab on a mode command while running queues it: applied at the next idle
/// boundary instead of being refused at admission.
#[test]
fn running_tab_mode_command_becomes_queue() {
    let command = "/sandbox later";
    let (action, input, _) = press_running_mode_command(command, KeyCode::Tab);
    assert!(matches!(action, KeyAction::Queue(text) if text == command));
    assert!(input.is_empty(), "queue clears the input line");
}

/// Enter on a mode command while a running subagent is focused stays blocked
/// (subagents have no agent-switch concept) with the input preserved.
#[test]
fn focused_subagent_enter_mode_command_still_blocked() {
    let command = "/act later";
    let (action, input, cursor) = press_running_command(command, KeyCode::Enter, true);
    assert!(matches!(action, KeyAction::ModeSwitchBlocked));
    assert_eq!(input, command);
    assert_eq!(cursor, command.chars().count());
}

/// Tab on a mode command while a subagent is focused is unsupported like any
/// other queue — the mode gate no longer takes priority.
#[test]
fn focused_subagent_tab_mode_command_unsupported() {
    let command = "/act later";
    let (action, input, _) = press_running_command(command, KeyCode::Tab, true);
    assert!(matches!(action, KeyAction::QueueUnsupported));
    assert_eq!(input, command);
}

#[test]
fn running_normal_prompt_keeps_steer_and_queue_behavior() {
    let (enter, input, _) = press_running_mode_command("continue", KeyCode::Enter);
    assert!(matches!(enter, KeyAction::Steer(text) if text == "continue"));
    assert!(input.is_empty());

    let (tab, input, _) = press_running_mode_command("later", KeyCode::Tab);
    assert!(matches!(tab, KeyAction::Queue(text) if text == "later"));
    assert!(input.is_empty());
}

/// Shift+Tab (BackTab) arms the clear-context countdown guard: it clears the
/// composer and forwards the draft as the compound rest of the canonical
/// command. Execution only happens after the confirm (Enter / window
/// elapsed); Esc 回撤 restores the draft. Identical entry to typing the
/// command.
#[test]
fn backtab_arms_clear_context_confirm() {
    fn run(input_text: &str) -> (KeyAction, String) {
        let mut input = input_text.to_string();
        let mut cursor = input.chars().count();
        let mut hist_idx = None;
        let mut scroll = 0;
        let mut follow = true;
        let mut last_esc = None;
        let mut skill_menu = None;
        let mut undo_state = crate::undo::init(&input, cursor);
        let mut queue_scroll = 0;
        let mut file_menu = None;
        let action = handle_key(
            KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE),
            &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
            &mut input,
            &mut cursor,
            &[],
            &mut hist_idx,
            false,
            "act",
            &mut scroll,
            &mut follow,
            &mut last_esc,
            &mut skill_menu,
            80,
            2,
            false,
            false,
            &mut undo_state,
            &mut queue_scroll,
            &mut file_menu,
            Path::new("."),
        );
        (action, input)
    }

    let (action, input) = run("");
    match action {
        KeyAction::ArmClearConfirm { rest } => assert_eq!(rest, None),
        other => panic!("expected ArmClearConfirm, got {other:?}"),
    }
    assert!(input.is_empty(), "arming clears the input line");

    let (action, input) = run("now run the checks");
    match action {
        KeyAction::ArmClearConfirm { rest } => {
            assert_eq!(rest, Some("now run the checks".into()))
        }
        other => panic!("expected compound Submit, got {other:?}"),
    }
    assert!(input.is_empty(), "submit clears the input line");
}
