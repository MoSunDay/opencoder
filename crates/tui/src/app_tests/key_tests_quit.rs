//! Quit / cancel key tests for `handle_key`. Extracted from `key_tests.rs`
//! to keep that file within the 800-line iteration cap.
//!
//! Covers the three terminal encodings of Ctrl+C (chord / raw ETX / Kitty)
//! and Ctrl+D, in both the normal composer path and the subagent-focus
//! (input-disabled) path: idle Ctrl+C quits (like Ctrl+D), while a running
//! Ctrl+C cancels the in-flight turn.

use super::*;

#[test]
fn ctrl_c_quits_when_idle() {
    // Idle Ctrl+C now quits, just like Ctrl+D.
    let mut input = String::new();
    let mut idx = 0;
    let action = run_handle(
        key(KeyCode::Char('c'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(
        matches!(action, KeyAction::Quit),
        "idle Ctrl+C must quit (same as Ctrl+D)"
    );
}

#[test]
fn ctrl_c_cancels_when_running() {
    // While a turn is running, Ctrl+C interrupts instead of quitting.
    let mut input = String::new();
    let mut idx = 0;
    let action = run_handle(
        key(KeyCode::Char('c'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        true,
        "act",
    );
    assert!(
        matches!(action, KeyAction::Cancel),
        "Ctrl+C while running must cancel, not quit"
    );
}

#[test]
fn ctrl_d_quits() {
    let mut input = String::new();
    let mut idx = 0;
    let action = run_handle(
        key(KeyCode::Char('d'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(matches!(action, KeyAction::Quit), "Ctrl+D must quit");
}

#[test]
fn raw_eot_quits() {
    // Some terminals/crossterm configs deliver Ctrl+D as a bare EOT control
    // char (0x04) without the CONTROL modifier — that path must still quit.
    let mut input = String::new();
    let mut idx = 0;
    let action = run_handle(
        key(KeyCode::Char('\u{4}'), KeyModifiers::NONE),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(
        matches!(action, KeyAction::Quit),
        "raw EOT (Ctrl+D) must quit"
    );
}

#[test]
fn raw_etx_quits_when_idle() {
    // Bare ETX (0x03) delivered by some terminals for Ctrl+C now quits when
    // idle, matching the Ctrl+D raw-EOT behaviour.
    let mut input = String::new();
    let mut idx = 0;
    let action = run_handle(
        key(KeyCode::Char('\u{3}'), KeyModifiers::NONE),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(
        matches!(action, KeyAction::Quit),
        "raw ETX (Ctrl+C) must quit when idle"
    );
}

#[test]
fn kitty_ctrl_d_quits() {
    // Under Kitty keyboard protocol (DISAMBIGUATE_ESCAPE_CODES) crossterm
    // reports Ctrl+D as Char('\u{4}') WITH the CONTROL modifier — this must
    // still quit (regression: was swallowed by the CONTROL match arm).
    let mut input = String::new();
    let mut idx = 0;
    let action = run_handle(
        key(KeyCode::Char('\u{4}'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(matches!(action, KeyAction::Quit), "Kitty Ctrl+D must quit");
}

#[test]
fn kitty_ctrl_c_quits_when_idle() {
    // Kitty-protocol path for Ctrl+C (Char('\u{3}') + CONTROL) now quits when
    // idle, same as the Ctrl+D Kitty path.
    let mut input = String::new();
    let mut idx = 0;
    let action = run_handle(
        key(KeyCode::Char('\u{3}'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        false,
        "act",
    );
    assert!(
        matches!(action, KeyAction::Quit),
        "Kitty Ctrl+C must quit when idle"
    );
}

#[test]
fn ctrl_c_quits_when_input_disabled() {
    // Subagent-focus view (input disabled): idle Ctrl+C must still quit, same
    // as the normal composer path and Ctrl+D. Only quit/help/scroll are
    // honoured while browsing a subagent, so Ctrl+C quitting here is the
    // desired escape hatch rather than a silent no-op.
    let mut input = String::new();
    let mut idx = 0;
    let action = run_handle_disabled(
        key(KeyCode::Char('c'), KeyModifiers::CONTROL),
        &mut input,
        &mut idx,
        "act",
    );
    assert!(
        matches!(action, KeyAction::Quit),
        "idle Ctrl+C must quit even when input is disabled (subagent-focus)"
    );
}
