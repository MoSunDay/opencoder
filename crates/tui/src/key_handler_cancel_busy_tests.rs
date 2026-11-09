//! Cancel-key regression tests for the subagent-drain busy window.
//!
//! During the drain window (`running == false` but `chat.subagents_running >
//! 0` — autopilot stage gap, cancel grace, reabsorb tail) the parent turn
//! already reports idle while the worker is still inside its turn. Both
//! abort chords (Ctrl+C and double-Esc) must treat live subagents as busy:
//! CANCEL the in-flight work, never degrade to Quit (Ctrl+C) or a silent
//! no-op (double-Esc).

use super::*;
use crossterm::event::{KeyCode, KeyModifiers};

fn bindings() -> crate::keymap::KeyBindings {
    crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default())
}

/// Press a key through `handle_key` with only the busy flags varied.
fn press_busy(
    code: KeyCode,
    modifiers: KeyModifiers,
    running: bool,
    subagents_running: bool,
    last_esc: &mut Option<std::time::Instant>,
) -> (KeyAction, String) {
    let mut input = "draft".to_string();
    let mut cursor = input.chars().count();
    let mut hist_idx = None;
    let mut scroll = 0;
    let mut follow = true;
    let mut skill_menu = None;
    let mut undo_state = crate::undo::init(&input, cursor);
    let mut queue_scroll = 0;
    let action = handle_key(
        KeyEvent::new(code, modifiers),
        &bindings(),
        &mut input,
        &mut cursor,
        &[],
        &mut hist_idx,
        running,
        subagents_running,
        "act",
        &mut scroll,
        &mut follow,
        last_esc,
        &mut skill_menu,
        80,
        2,
        false, // subagent_focused
        false, // sidecar_focused
        false, // input_disabled
        &mut undo_state,
        &mut queue_scroll,
        &mut None,
        None,
    );
    (action, input)
}

/// Ctrl+C during the drain window (idle parent + live subagents) must
/// CANCEL, not quit: gating on `running` alone downgraded the abort to a
/// Quit exactly when the user only wanted to stop the draining batch.
#[test]
fn ctrl_c_during_subagent_drain_cancels_instead_of_quitting() {
    let (action, _input) =
        press_busy(KeyCode::Char('c'), KeyModifiers::CONTROL, false, true, &mut None);
    assert!(matches!(action, KeyAction::Cancel), "got {action:?}");
}

/// Double-Esc during the drain window must hard-abort (Cancel): gating on
/// `running` alone made the chord a silent no-op (first Esc cleared the
/// draft, second armed nothing).
#[test]
fn double_esc_during_subagent_drain_cancels() {
    let mut last_esc = None;
    // First Esc: arms the window (and clears the draft) — not yet a cancel.
    let (first, input) = press_busy(KeyCode::Esc, KeyModifiers::NONE, false, true, &mut last_esc);
    assert!(matches!(first, KeyAction::None), "got {first:?}");
    assert_eq!(input, "", "single Esc clears the draft");
    // Second Esc inside the 350ms window: hard abort.
    let (second, _input) =
        press_busy(KeyCode::Esc, KeyModifiers::NONE, false, true, &mut last_esc);
    assert!(matches!(second, KeyAction::Cancel), "got {second:?}");
}

/// Control: with no work in flight at all, Ctrl+C keeps its Quit meaning.
#[test]
fn ctrl_c_when_fully_idle_still_quits() {
    let (action, _) =
        press_busy(KeyCode::Char('c'), KeyModifiers::CONTROL, false, false, &mut None);
    assert!(matches!(action, KeyAction::Quit), "got {action:?}");
}

/// Control: double-Esc on a fully idle composer never escalates to Cancel.
#[test]
fn double_esc_when_fully_idle_stays_input_clear() {
    let mut last_esc = None;
    let (first, _) = press_busy(KeyCode::Esc, KeyModifiers::NONE, false, false, &mut last_esc);
    assert!(matches!(first, KeyAction::None));
    let (second, _) = press_busy(KeyCode::Esc, KeyModifiers::NONE, false, false, &mut last_esc);
    assert!(matches!(second, KeyAction::None), "got {second:?}");
}
