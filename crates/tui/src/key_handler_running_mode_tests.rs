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
        false, // sidecar_focused
        false,
        &mut undo_state,
        &mut queue_scroll,
        &mut file_menu,
        Path::new("."));
    (action, input, cursor)
}

fn press_running_mode_command(command: &str, code: KeyCode) -> (KeyAction, String, usize) {
    press_running_command(command, code, false)
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
    let mut file_menu = None;
    let action = handle_key(
        KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL),
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
        &mut input,
        &mut cursor,
        &[],
        &mut hist_idx,
        running,
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
        &mut file_menu,
        Path::new("."));
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

/// Enter on a mode command while the parent runs admits it as a steer: the
/// runner applies it at the next turn boundary (delayed application).
#[test]
fn running_enter_mode_command_becomes_steer() {
    for command in [
        "/plan",
        "/act",
        "/plan review this",
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
    let command = "/plan later";
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

/// Shift+Tab (BackTab) in plan mode arms the clear-context countdown guard:
/// it clears the composer and forwards the draft as the compound rest of the
/// canonical command, carrying the raw text as `draft` for the guard's Esc
/// 回撤 to restore verbatim. Execution only happens after the confirm (Enter
/// / window elapsed). Identical entry to typing the command.
#[test]
fn backtab_in_plan_mode_arms_clear_context_confirm() {
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
            "plan",
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
            &mut file_menu,
            Path::new("."));
        (action, input)
    }

    let (action, input) = run("");
    match action {
        KeyAction::ArmClearConfirm { rest, draft } => {
            assert_eq!(rest, None);
            assert_eq!(draft, None);
        }
        other => panic!("expected ArmClearConfirm, got {other:?}"),
    }
    assert!(input.is_empty(), "arming clears the input line");

    let (action, input) = run("now run the checks");
    match action {
        KeyAction::ArmClearConfirm { rest, draft } => {
            assert_eq!(rest, Some("now run the checks".into()));
            assert_eq!(draft, Some("now run the checks".into()));
        }
        other => panic!("expected compound Submit, got {other:?}"),
    }
    assert!(input.is_empty(), "submit clears the input line");

    // Raw draft round-trips untrimmed — Esc 回撤 restores byte-exact input.
    let (action, _) = run("  padded draft  ");
    match action {
        KeyAction::ArmClearConfirm { rest, draft } => {
            assert_eq!(rest, Some("padded draft".into()));
            assert_eq!(draft, Some("  padded draft  ".into()));
        }
        other => panic!("expected ArmClearConfirm, got {other:?}"),
    }
}

/// BackTab-arm → Esc 回撤 is lossless: the draft captured by the guard's arm
/// action comes back into the composer verbatim when the countdown is
/// cancelled (handle_key → arm(rest, draft) → intercept(Esc), the exact
/// chain app.rs runs).
#[test]
fn backtab_arm_then_esc_restores_the_raw_draft() {
    for raw in ["fold and then run the checks", "  padded  ", ""] {
        let mut input = raw.to_string();
        let mut cursor = raw.chars().count();
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
            "plan",
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
            &mut file_menu,
            Path::new("."));
        let (rest, draft) = match action {
            KeyAction::ArmClearConfirm { rest, draft } => (rest, draft),
            other => panic!("expected ArmClearConfirm, got {other:?}"),
        };
        assert!(input.is_empty(), "arming clears the composer");

        // The guard's Esc puts the raw draft back — nothing was lost.
        let mut cc = Some(crate::clear_confirm::arm(rest, draft));
        let mut undo = crate::undo::init(&input, cursor);
        assert_eq!(
            crate::clear_confirm::intercept(
                &mut cc,
                &mut input,
                &mut cursor,
                &mut undo,
                KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
            ),
            Some(crate::clear_confirm::ConfirmFlow::Cancel)
        );
        assert!(cc.is_none(), "Esc drops the arm");
        assert_eq!(input, raw, "Esc 回撤 restores the raw draft verbatim");
        assert_eq!(cursor, raw.chars().count());
    }
}

/// Shift+Tab in act mode is the non-destructive way back to plan: it issues a
/// plain mode switch (context preserved), not the clear-context countdown.
/// The composer draft is untouched — unlike the plan-mode arm, nothing is
/// swallowed.
#[test]
fn backtab_in_act_mode_switches_to_plan() {
    fn run(input_text: &str, running: bool) -> (KeyAction, String) {
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
            running,
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
            &mut file_menu,
            Path::new("."));
        (action, input)
    }

    for running in [false, true] {
        let (action, input) = run("some draft", running);
        assert!(
            matches!(action, KeyAction::SwitchAgent(ref to) if to == "plan"),
            "act + Shift+Tab must switch to plan, got {action:?}"
        );
        assert_eq!(input, "some draft", "mode switch preserves the composer");
    }
}

// ----- Shift+Tab chord hardening: focus guard, modifier filter, spellings -----

/// Drive Shift+Tab (either spelling) through `handle_key` with explicit
/// agent / focus / modifier knobs — the shared harness for the chord tests.
#[allow(clippy::too_many_arguments)]
fn press_shift_tab(
    agent: &str,
    code: KeyCode,
    mods: KeyModifiers,
    running: bool,
    subagent_focused: bool,
    sidecar_focused: bool,
) -> (KeyAction, String) {
    let mut input = "steer text".to_string();
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
        KeyEvent::new(code, mods),
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
        &mut input,
        &mut cursor,
        &[],
        &mut hist_idx,
        running,
        agent,
        &mut scroll,
        &mut follow,
        &mut last_esc,
        &mut skill_menu,
        80,
        2,
        subagent_focused,
        sidecar_focused,
        false,
        &mut undo_state,
        &mut queue_scroll,
        &mut file_menu,
        Path::new("."));
    (action, input)
}

/// F1: the plan-mode arm is a parent-session operation — while a running
/// subagent (or the sidecar box) is focused the chord must stay inert with
/// the draft intact, otherwise the armed guard would swallow the next Enter
/// and merge the pane's steer/ask text into the destructive clear command.
#[test]
fn backtab_arm_never_arms_when_subagent_or_sidecar_focused() {
    for (subagent_focused, sidecar_focused) in [(true, false), (false, true), (true, true)] {
        let (action, input) = press_shift_tab(
            "plan",
            KeyCode::BackTab,
            KeyModifiers::NONE,
            true,
            subagent_focused,
            sidecar_focused,
        );
        assert!(
            matches!(action, KeyAction::None),
            "focused pane ({subagent_focused}, {sidecar_focused}) must not arm, got {action:?}"
        );
        assert_eq!(input, "steer text", "inert chord keeps the draft");
    }
}

/// F2: the retired ctrl+shift+tab lands as BackTab+CONTROL|SHIFT on many
/// terminals — it must neither arm (plan) nor switch (act). Alt/Super
/// mutations are filtered the same way.
#[test]
fn backtab_with_ctrl_alt_super_never_arms_or_switches() {
    let ctrl_shift = KeyModifiers::CONTROL | KeyModifiers::SHIFT;
    for mods in [
        ctrl_shift,
        KeyModifiers::ALT | KeyModifiers::SHIFT,
        KeyModifiers::SUPER | KeyModifiers::SHIFT,
        KeyModifiers::CONTROL,
    ] {
        for agent in ["plan", "act"] {
            let (action, input) =
                press_shift_tab(agent, KeyCode::BackTab, mods, false, false, false);
            assert!(
                matches!(action, KeyAction::None),
                "agent={agent} mods={mods:?} must stay inert, got {action:?}"
            );
            assert_eq!(input, "steer text", "inert chord keeps the draft");
        }
    }
}

/// F7: terminals that report Shift+Tab as (Tab, SHIFT) get the same
/// mode-aware action as BackTab — arm in plan mode (payload identical).
#[test]
fn tab_shift_spelling_arms_like_backtab_in_plan_mode() {
    let (action, input) = press_shift_tab(
        "plan",
        KeyCode::Tab,
        KeyModifiers::SHIFT,
        false,
        false,
        false,
    );
    match action {
        KeyAction::ArmClearConfirm { rest, draft } => {
            assert_eq!(rest, Some("steer text".into()));
            assert_eq!(draft, Some("steer text".into()));
        }
        other => panic!("expected ArmClearConfirm, got {other:?}"),
    }
    assert!(input.is_empty(), "arming clears the input line");
}

/// F7: (Tab, SHIFT) in act mode switches to plan exactly like BackTab.
#[test]
fn tab_shift_spelling_switches_to_plan_in_act_mode() {
    for running in [false, true] {
        let (action, input) = press_shift_tab(
            "act",
            KeyCode::Tab,
            KeyModifiers::SHIFT,
            running,
            false,
            false,
        );
        assert!(
            matches!(action, KeyAction::SwitchAgent(ref to) if to == "plan"),
            "act + (Tab, SHIFT) must switch to plan, got {action:?}"
        );
        assert_eq!(input, "steer text", "mode switch preserves the composer");
    }
}

/// F7: (Tab, SHIFT) honours the same focus guard — a focused running
/// subagent must not get its pane hijacked by the parent arm.
#[test]
fn tab_shift_spelling_never_arms_when_subagent_focused() {
    let (action, input) = press_shift_tab(
        "plan",
        KeyCode::Tab,
        KeyModifiers::SHIFT,
        true,
        true,
        false,
    );
    assert!(matches!(action, KeyAction::None), "got {action:?}");
    assert_eq!(input, "steer text");

    // (Tab, SHIFT) with a Ctrl mutation is NOT the chord: handle_key's
    // "swallow any remaining Ctrl+key" guard stops it long before the Tab
    // arms — so it can neither arm nor queue.
    let (action, input) = press_shift_tab(
        "plan",
        KeyCode::Tab,
        KeyModifiers::SHIFT | KeyModifiers::CONTROL,
        true,
        false,
        false,
    );
    assert!(
        matches!(action, KeyAction::None),
        "ctrl-mutated (Tab, SHIFT) must stay inert, got {action:?}"
    );
    assert_eq!(input, "steer text", "the draft survives");
}
