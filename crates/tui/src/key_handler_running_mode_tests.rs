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
    let action = handle_key(
        KeyEvent::new(code, KeyModifiers::NONE),
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
        &mut input,
        &mut cursor,
        &[],
        &mut hist_idx,
        true,
        false,
        "act",
        &mut scroll,
        &mut follow,
        &mut last_esc,
        &mut skill_menu,
        80,
        2,
        subagent_focused,
        false, // sidecar_focused
        false,
        &mut undo_state,
        &mut queue_scroll,
        &mut None,
    );
    (action, input, cursor)
}

fn press_running_mode_command(command: &str, code: KeyCode) -> (KeyAction, String, usize) {
    press_running_command(command, code, false)
}

#[test]
fn agent_selection_intercepts_enter_and_tab_even_while_running() {
    for command in ["/agent", "/agent self", "/agent server-operator"] {
        for code in [KeyCode::Enter, KeyCode::Tab] {
            let (action, input, cursor) = press_running_mode_command(command, code);
            let KeyAction::SelectAgent(name) = action else {
                panic!("expected selection")
            };
            assert_eq!(name, command.strip_prefix("/agent").unwrap().trim());
            assert_eq!(input, "");
            assert_eq!(cursor, 0);
        }
    }
}

fn press_ctrl_t(agent: &str, running: bool, input_disabled: bool) -> (KeyAction, String) {
    let mut input = "draft stays".to_string();
    let mut cursor = input.chars().count();
    let mut hist_idx = None;
    let mut scroll = 0;
    let mut follow = true;
    let mut last_esc = None;
    let mut skill_menu = None;
    let mut undo_state = crate::undo::init(&input, cursor);
    let mut queue_scroll = 0;
    let action = handle_key(
        KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL),
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
        &mut input,
        &mut cursor,
        &[],
        &mut hist_idx,
        running,
        false,
        agent,
        &mut scroll,
        &mut follow,
        &mut last_esc,
        &mut skill_menu,
        80,
        2,
        input_disabled,
        false,
        input_disabled,
        &mut undo_state,
        &mut queue_scroll,
        &mut None,
    );
    (action, input)
}

#[test]
fn ctrl_t_toggles_act_and_plan_without_touching_draft() {
    for (agent, expected) in [("act", "plan"), ("plan", "act")] {
        let (action, input) = press_ctrl_t(agent, false, false);
        assert!(matches!(action, KeyAction::SwitchAgent(ref to) if to == expected));
        assert_eq!(input, "draft stays", "mode toggle preserves the composer");
    }
}

#[test]
fn ctrl_t_reaches_app_gate_while_running_or_subagent_focused() {
    for (running, input_disabled) in [(true, false), (false, true)] {
        let (action, input) = press_ctrl_t("plan", running, input_disabled);
        assert!(matches!(action, KeyAction::SwitchAgent(ref to) if to == "act"));
        assert_eq!(input, "draft stays");
    }
}

/// Enter on a BARE act/plan switch while the parent runs is refused with
/// `ModeSwitchBlocked`: a mode switch never applies mid-flight and is never
/// queued. The typed text stays so the user can retry when idle.
#[test]
fn running_enter_bare_mode_switch_blocked() {
    for command in ["/plan", "/act"] {
        let (action, input, cursor) = press_running_mode_command(command, KeyCode::Enter);
        assert!(matches!(action, KeyAction::ModeSwitchBlocked), "{command}");
        assert_eq!(input, command, "blocked switch keeps the input line");
        assert_eq!(cursor, command.chars().count());
    }
}

/// Enter on a COMPOUND mode command while the parent runs still steers: it
/// is a task submission whose mode switch rides along at the idle boundary
/// (the runner applies the command at the next turn boundary).
#[test]
fn running_enter_compound_mode_command_becomes_steer() {
    for command in ["/plan review this", "/clear_context now"] {
        let (action, input, _) = press_running_mode_command(command, KeyCode::Enter);
        assert!(matches!(action, KeyAction::Steer(text) if text == command));
        assert!(input.is_empty(), "steer clears the input line");
    }
}

/// Tab on a BARE act/plan switch while running is refused like Enter: it
/// would otherwise queue a mid-flight switch that lands unannounced at the
/// idle boundary. The typed text stays.
#[test]
fn running_tab_bare_mode_switch_blocked() {
    for command in ["/plan", "/act"] {
        let (action, input, _) = press_running_mode_command(command, KeyCode::Tab);
        assert!(matches!(action, KeyAction::ModeSwitchBlocked), "{command}");
        assert_eq!(input, command, "blocked switch keeps the input line");
    }
}

/// Tab on a compound mode command while running still queues it: applied at
/// the next idle boundary as part of the task submission.
#[test]
fn running_tab_compound_mode_command_becomes_queue() {
    let command = "/plan later";
    let (action, input, _) = press_running_mode_command(command, KeyCode::Tab);
    assert!(matches!(action, KeyAction::Queue(text) if text == command));
    assert!(input.is_empty(), "queue clears the input line");
}

/// Bug case: the parent turn already reports idle (`running = false`) while
/// its subagent batch is still draining — autopilot stage gap (PLAN Done →
/// ACT dispatches task subagents), cancel grace window, or reabsorb tail.
/// Tab must still queue: a submit would bypass the queue row and start a new
/// run immediately, betraying the follow-up intent.
#[test]
fn idle_tab_with_live_subagents_becomes_queue() {
    let mut input = "after the subagents finish".to_string();
    let mut cursor = input.chars().count();
    let mut hist_idx = None;
    let mut scroll = 0;
    let mut follow = true;
    let mut last_esc = None;
    let mut skill_menu = None;
    let mut undo_state = crate::undo::init(&input, cursor);
    let mut queue_scroll = 0;
    let action = handle_key(
        KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
        &mut input,
        &mut cursor,
        &[],
        &mut hist_idx,
        false,
        true, // subagents_running: live subagents keep Tab on the queue arm
        "act",
        &mut scroll,
        &mut follow,
        &mut last_esc,
        &mut skill_menu,
        80,
        2,
        false,
        false,
        false,
        &mut undo_state,
        &mut queue_scroll,
        &mut None,
    );
    assert!(
        matches!(action, KeyAction::Queue(ref t) if t == "after the subagents finish"),
        "idle + live subagents must queue, got {action:?}"
    );
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

/// Shift+Tab (BackTab) in plan mode arms the clear-context countdown guard:
/// it clears the composer and forwards the draft as the compound rest of the
/// canonical command, carrying the raw text as `draft` for the guard's Esc
/// 回撤 to restore verbatim. Execution only happens after the confirm (Enter
/// / window elapsed). Identical entry to typing the command.

#[test]
fn task_command_opens_picker_instead_of_steering_running_execution() {
    for command in ["/task", "/tasks", "/t"] {
        for code in [KeyCode::Enter, KeyCode::Tab] {
            let (action, input, cursor) = press_running_mode_command(command, code);
            assert!(matches!(action, KeyAction::OpenTask));
            assert!(input.is_empty());
            assert_eq!(cursor, 0);
        }
    }
}

#[path = "key_handler_running_mode_tests/shift_tab.rs"]
mod shift_tab;
