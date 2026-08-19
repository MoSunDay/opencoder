//! Mode-switch keys in the subagent-focus (input-disabled) view. Leaving or
//! switching mode must never be blocked by view state: switch_mode_clear /
//! switch_mode_keep / switch_mode bindings and a raw BackTab all stay live
//! and funnel into `handle_switch_agent`, whose running gate blocks both
//! directions while busy (plan→act and act→plan alike get the busy hint).
//! Split out of `key_handler_tests.rs` to keep that file under the 800-line
//! cap.

use super::*;

/// Drive `handle_key` once in the input-disabled (subagent-focus) state with
/// the default keymap, mirroring the full 18-argument harness call. `input`
/// is passed through so callers can assert it is preserved.
fn disabled_mode_key(ev: KeyEvent, agent: &str, input: &mut String) -> KeyAction {
    let bindings = crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default());
    let mut cursor = input.chars().count();
    let history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;
    let mut scroll = 0u32;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut skill_menu: Option<SkillMenu> = None;
    let mut undo_state = crate::undo::init(input, cursor);
    let mut queue_scroll: u32 = 0;
    handle_key(
        ev,
        &bindings,
        input,
        &mut cursor,
        &history,
        &mut hist_idx,
        false,
        agent,
        &mut scroll,
        &mut follow,
        &mut last_esc,
        &mut skill_menu,
        80,
        2,
        false,
        true,
        &mut undo_state,
        &mut queue_scroll,
    )
}

/// Raw BackTab (Shift+Tab) toggles the mode in BOTH directions from the
/// input-disabled view.
#[test]
fn handle_key_disabled_allows_backtab_mode_switch() {
    let mut input = String::new();
    let action = disabled_mode_key(
        KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE),
        "plan",
        &mut input,
    );
    match action {
        KeyAction::SwitchAgent(n) => assert_eq!(n, "act"),
        other => panic!("expected SwitchAgent(act) from plan, got {other:?}"),
    }

    let action = disabled_mode_key(
        KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE),
        "act",
        &mut input,
    );
    match action {
        KeyAction::SwitchAgent(n) => assert_eq!(n, "plan"),
        other => panic!("expected SwitchAgent(plan) from act, got {other:?}"),
    }
}

/// A prefilled `/plan <content>` compound is NOT submitted from the disabled
/// view — that branch belongs to the enabled path only. The raw BackTab
/// toggles the agent and the input is preserved verbatim.
#[test]
fn handle_key_disabled_backtab_skips_plan_compound_submit() {
    let mut input = "/plan do the thing".to_string();
    let action = disabled_mode_key(
        KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE),
        "act",
        &mut input,
    );
    match action {
        KeyAction::SwitchAgent(n) => assert_eq!(n, "plan"),
        other => panic!("expected SwitchAgent(plan), got {other:?}"),
    }
    assert_eq!(
        input, "/plan do the thing",
        "compound-submit branch must be skipped: input preserved"
    );
}

/// The bound mode-switch keys stay live in the disabled view:
/// switch_mode_clear (default alt+tab, matches Tab/BackTab + ALT) returns
/// `SwitchAgent`, and switch_mode_keep (default ctrl+shift+tab, matches
/// BackTab + CONTROL and Tab + CONTROL|SHIFT) returns `SwitchAgentNoClear`.
/// Event variants mirror the keymap tests for these two bindings.
#[test]
fn handle_key_disabled_allows_bound_mode_switch_keys() {
    let mut input = String::new();

    // switch_mode_clear: Tab + ALT (and the terminal BackTab + ALT variant).
    for ev in [
        KeyEvent::new(KeyCode::Tab, KeyModifiers::ALT),
        KeyEvent::new(KeyCode::BackTab, KeyModifiers::ALT),
    ] {
        match disabled_mode_key(ev, "plan", &mut input) {
            KeyAction::SwitchAgent(n) => assert_eq!(n, "act"),
            other => panic!("alt+tab must switch mode, got {other:?}"),
        }
    }

    // switch_mode_keep: BackTab + CONTROL (and kitty Tab + CONTROL|SHIFT).
    for ev in [
        KeyEvent::new(KeyCode::BackTab, KeyModifiers::CONTROL),
        KeyEvent::new(KeyCode::Tab, KeyModifiers::CONTROL | KeyModifiers::SHIFT),
    ] {
        match disabled_mode_key(ev, "plan", &mut input) {
            KeyAction::SwitchAgentNoClear(n) => assert_eq!(n, "act"),
            other => panic!("ctrl+shift+tab must switch mode, got {other:?}"),
        }
    }
}

/// `switch_mode` (default ctrl+t) must ALSO stay live in the input-disabled
/// (subagent-focus) view: it returns `SwitchAgentNoClear` exactly like the
/// enabled path — the whitelist omission made the user-customized chord dead
/// while a subagent was focused. Both toggle directions are covered.
#[test]
fn handle_key_disabled_allows_switch_mode_binding() {
    let mut input = String::new();

    // plan -> act
    match disabled_mode_key(
        KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL),
        "plan",
        &mut input,
    ) {
        KeyAction::SwitchAgentNoClear(n) => assert_eq!(n, "act"),
        other => panic!("ctrl+t must switch mode from the disabled view, got {other:?}"),
    }

    // act -> plan
    match disabled_mode_key(
        KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL),
        "act",
        &mut input,
    ) {
        KeyAction::SwitchAgentNoClear(n) => assert_eq!(n, "plan"),
        other => panic!("ctrl+t must switch mode from the disabled view, got {other:?}"),
    }

    // Input stays untouched — no composer mutation from the disabled view.
    assert!(input.is_empty());
}
