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
        &mut file_menu,
        Path::new("."),
    );
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
        &mut file_menu,
        Path::new("."),
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

/// Enter on a mode command while the parent runs admits it as a steer: the
/// runner applies it at the next turn boundary (delayed application).
#[test]
fn running_enter_mode_command_becomes_steer() {
    for command in ["/plan", "/act", "/plan review this", "/clear_context now"] {
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
    let mut file_menu = None;
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
        &mut file_menu,
        Path::new("."),
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
            Path::new("."),
        );
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
            Path::new("."),
        );
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
            false,
            &mut undo_state,
            &mut queue_scroll,
            &mut file_menu,
            Path::new("."),
        );
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

/// Shift+Tab spelled as (Tab, SHIFT) — some terminals report the chord this
/// way — must take the same mode-aware path
/// as BackTab: arm the countdown in plan mode, switch to plan in act mode.
/// The plain Tab arm (queue/submit) must never swallow the chord.
#[test]
fn tab_shift_spelling_arms_or_switches_like_backtab() {
    fn run(
        code: KeyCode,
        mods: KeyModifiers,
        agent: &str,
        input_text: &str,
    ) -> (KeyAction, String) {
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
            KeyEvent::new(code, mods),
            &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
            &mut input,
            &mut cursor,
            &[],
            &mut hist_idx,
            false,
            false,
            agent,
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
            Path::new("."),
        );
        (action, input)
    }

    // Plan mode: both spellings arm the countdown, forwarding the draft.
    for code in [KeyCode::BackTab, KeyCode::Tab] {
        let (action, input) = run(code, KeyModifiers::SHIFT, "plan", "run the checks");
        match action {
            KeyAction::ArmClearConfirm { rest, draft } => {
                assert_eq!(rest, Some("run the checks".into()), "{code:?}");
                assert_eq!(draft, Some("run the checks".into()), "{code:?}");
            }
            other => panic!("{code:?} in plan mode must arm, got {other:?}"),
        }
        assert!(input.is_empty(), "{code:?} arming clears the composer");
    }

    // Act mode: both spellings switch back to plan, draft preserved.
    for code in [KeyCode::BackTab, KeyCode::Tab] {
        let (action, input) = run(code, KeyModifiers::SHIFT, "act", "draft stays");
        assert!(
            matches!(action, KeyAction::SwitchAgent(ref to) if to == "plan"),
            "{code:?} in act mode must switch to plan, got {action:?}"
        );
        assert_eq!(
            input, "draft stays",
            "{code:?} switch preserves the composer"
        );
    }
}

/// CONTROL/ALT/SUPER chord variants of the Shift+Tab family never arm the
/// countdown nor switch modes: the retired ctrl+shift+tab (BackTab+CONTROL)
/// must stay inert, mirroring the confirm side (`clear_confirm::intercept`)
/// — a chord that cannot confirm the guard must not arm it either.
#[test]
fn ctrl_alt_shift_tab_chords_never_arm_or_switch() {
    fn run(code: KeyCode, mods: KeyModifiers, agent: &str) -> (KeyAction, String) {
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
            KeyEvent::new(code, mods),
            &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
            &mut input,
            &mut cursor,
            &[],
            &mut hist_idx,
            false,
            false,
            agent,
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
            Path::new("."),
        );
        (action, input)
    }

    // BackTab + CONTROL/ALT/SUPER: fully inert in both modes — no arm, no
    // switch, draft untouched.
    for mods in [
        KeyModifiers::CONTROL,
        KeyModifiers::ALT,
        KeyModifiers::SUPER,
    ] {
        for agent in ["plan", "act"] {
            let (action, input) = run(KeyCode::BackTab, mods, agent);
            assert!(
                matches!(action, KeyAction::None),
                "BackTab+{mods:?} in {agent} must stay inert, got {action:?}"
            );
            assert_eq!(input, "draft stays", "inert chord preserves the draft");
        }
    }

    // (Tab, SHIFT+CONTROL) misses the shift_tab arm and is swallowed by the
    // generic Ctrl-combo guard — never an arm, never a mode switch, and it
    // can never leak into the plain Tab queue/submit arm either.
    let (action, input) = run(
        KeyCode::Tab,
        KeyModifiers::SHIFT | KeyModifiers::CONTROL,
        "plan",
    );
    assert!(
        matches!(action, KeyAction::None),
        "Tab+SHIFT+CONTROL must be swallowed inert, got {action:?}"
    );
    assert_eq!(
        input, "draft stays",
        "the swallowed chord preserves the draft"
    );
}

/// The arm is a PARENT-session operation: while a running subagent (or the
/// sidecar box) is focused, Shift+Tab must not arm — the armed guard would
/// swallow the next Enter meant for the focused pane. The chord is inert
/// there, mirroring the plain Tab arm's QueueUnsupported gate.
#[test]
fn shift_tab_with_focused_subagent_never_arms() {
    for code in [KeyCode::BackTab, KeyCode::Tab] {
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
            KeyEvent::new(code, KeyModifiers::SHIFT),
            &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
            &mut input,
            &mut cursor,
            &[],
            &mut hist_idx,
            true,
            false,
            "plan",
            &mut scroll,
            &mut follow,
            &mut last_esc,
            &mut skill_menu,
            80,
            2,
            true,  // subagent_focused
            false, // sidecar_focused
            false,
            &mut undo_state,
            &mut queue_scroll,
            &mut file_menu,
            Path::new("."),
        );
        assert!(
            matches!(action, KeyAction::None),
            "{code:?} with a focused subagent must stay inert, got {action:?}"
        );
        assert_eq!(input, "steer text", "the draft stays for the child steer");
    }
}
