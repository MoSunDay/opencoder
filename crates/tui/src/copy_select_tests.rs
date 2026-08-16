//! Tests for [`crate::copy_select`] — key routing (migrated from the old
//! `copy_mode` module), movement/clamping, wrapped-line rejoin yanks,
//! decoration stripping and highlight styling.

use super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use opencoder_core::Config;
use ratatui::text::Line;

use crate::chat::ChatView;
use crate::keymap::KeyBindings;

fn keybindings() -> KeyBindings {
    KeyBindings::from_config(&Config::default())
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn plain(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

/// A view whose flattened lines are exactly `lines` (one Marker block per
/// line) — markers render verbatim, independent of the markdown renderer.
fn view_from_lines(lines: &[&str]) -> ChatView {
    let mut v = ChatView::default();
    for &l in lines {
        v.push_marker(Line::from(l.to_string()));
    }
    v
}

/// Viewport over `lines` word-wrapped at `width`.
fn cache(lines: &[&str], width: u16) -> ViewportCache {
    ViewportCache::build(&view_from_lines(lines), width, 0, 0)
}

// ── is_active / entry-phase state ───────────────────────────────────────────

#[test]
fn is_active_truth_table() {
    let sel = CopySel::entry(0);
    assert!(!is_active(None, false));
    assert!(is_active(Some(&sel), false));
    assert!(is_active(None, true));
    assert!(is_active(Some(&sel), true));
}

#[test]
fn entry_state_is_pristine() {
    let s = CopySel::entry(7);
    assert_eq!(s.cursor, 7);
    assert!(s.anchor.is_none());
    assert!(s.copied_at.is_none());
    assert!(!s.selecting());
    assert_eq!(s.row_range(), None);
}

#[test]
fn row_range_normalizes_reversed_selection() {
    let s = CopySel {
        cursor: 2,
        anchor: Some(9),
        copied_at: None,
    };
    assert_eq!(s.row_range(), Some((2, 9)));
}

// ── key routing (migrated from copy_mode tests) ─────────────────────────────

#[test]
fn toggle_key_enters_when_inactive() {
    let kb = keybindings();
    let c = cache(&["a", "b"], 40);
    let mut sel = None;
    let (mut scroll, mut follow) = (3u32, true);
    assert_eq!(
        handle_key(&ctrl('g'), &mut sel, &kb, Some(&c), 5, &mut scroll, &mut follow),
        CopyOutcome::Consumed
    );
    let s = sel.expect("mode entered");
    assert_eq!(s.cursor, 3, "cursor parks on the scroll top");
    assert!(s.anchor.is_none());
}

#[test]
fn toggle_key_exits_when_active() {
    let kb = keybindings();
    let c = cache(&["a"], 40);
    let mut sel = Some(CopySel::entry(0));
    let (mut scroll, mut follow) = (0u32, false);
    assert_eq!(
        handle_key(&ctrl('g'), &mut sel, &kb, Some(&c), 5, &mut scroll, &mut follow),
        CopyOutcome::Exit
    );
    assert!(sel.is_some(), "caller clears the mode on Exit");
}

#[test]
fn active_mode_swallows_other_keys() {
    let kb = keybindings();
    let c = cache(&["a"], 40);
    let mut sel = Some(CopySel::entry(0));
    let (mut scroll, mut follow) = (0u32, false);
    for k in [plain('x'), plain('a'), key(KeyCode::F(5))] {
        assert_eq!(
            handle_key(&k, &mut sel, &kb, Some(&c), 5, &mut scroll, &mut follow),
            CopyOutcome::Consumed,
            "active mode must swallow {k:?}"
        );
        assert!(sel.is_some());
    }
}

#[test]
fn esc_and_q_exit_active_mode() {
    let kb = keybindings();
    let c = cache(&["a"], 40);
    for k in [key(KeyCode::Esc), plain('q')] {
        let mut sel = Some(CopySel::entry(0));
        let (mut scroll, mut follow) = (0u32, false);
        assert_eq!(
            handle_key(&k, &mut sel, &kb, Some(&c), 5, &mut scroll, &mut follow),
            CopyOutcome::Exit
        );
    }
}

#[test]
fn inactive_passes_through_non_toggle_keys() {
    let kb = keybindings();
    let c = cache(&["a"], 40);
    let mut sel = None;
    let (mut scroll, mut follow) = (0u32, false);
    assert_eq!(
        handle_key(&plain('x'), &mut sel, &kb, Some(&c), 5, &mut scroll, &mut follow),
        CopyOutcome::Ignored
    );
    assert!(sel.is_none(), "mode must not be entered by other keys");
}

#[test]
fn toggle_on_empty_viewport_flashes_instead_of_ignoring() {
    let kb = keybindings();
    let empty = ViewportCache::build(&ChatView::default(), 40, 0, 0);
    let mut sel = None;
    let (mut scroll, mut follow) = (0u32, true);
    assert_eq!(
        handle_key(&ctrl('g'), &mut sel, &kb, Some(&empty), 5, &mut scroll, &mut follow),
        CopyOutcome::Empty
    );
    assert!(sel.is_none(), "mode must not be entered with nothing to select");
    assert_eq!((scroll, follow), (0u32, true), "state must be untouched");
    // Tutorial-screen case: the cache was never built at all.
    assert_eq!(
        handle_key(&ctrl('g'), &mut sel, &kb, None, 5, &mut scroll, &mut follow),
        CopyOutcome::Empty
    );
    assert!(sel.is_none());
    assert_eq!((scroll, follow), (0u32, true));
}

#[test]
fn empty_flash_text_is_user_facing() {
    assert!(!EMPTY_FLASH_TEXT.is_empty());
    assert!(EMPTY_FLASH_TEXT.contains("nothing to copy"));
}

#[test]
fn dispatch_key_flashes_on_empty_and_passes_through() {
    let kb = keybindings();
    let mut sel = None;
    let (mut scroll, mut follow) = (0u32, true);
    let mut flash: Option<(String, u32)> = None;
    // Non-toggle key passes through without touching the flash.
    assert!(!dispatch_key(
        &plain('x'), &mut sel, &kb, None, 5, &mut scroll, &mut follow, &mut flash, 7,
    ));
    assert!(flash.is_none(), "pass-through must not flash");
    // Empty-transcript toggle is consumed and sets the feedback flash.
    assert!(dispatch_key(
        &ctrl('g'), &mut sel, &kb, None, 5, &mut scroll, &mut follow, &mut flash, 7,
    ));
    assert_eq!(flash.as_ref().map(|(t, _)| t.as_str()), Some(EMPTY_FLASH_TEXT));
    assert_eq!(flash.as_ref().map(|(_, t)| *t), Some(7), "flash stamps anim_tick");
    assert!(sel.is_none(), "empty toggle must not enter the mode");
    // Non-empty viewport: toggle enters without setting any flash, second
    // toggle exits (fresh flash slot proves nothing was written).
    let c = cache(&["a"], 40);
    let (mut scroll2, mut follow2) = (0u32, true);
    let mut flash2: Option<(String, u32)> = None;
    assert!(dispatch_key(
        &ctrl('g'), &mut sel, &kb, Some(&c), 5, &mut scroll2, &mut follow2, &mut flash2, 7,
    ));
    assert!(sel.is_some());
    assert!(flash2.is_none(), "entering on content must not flash");
    assert!(dispatch_key(
        &ctrl('g'), &mut sel, &kb, Some(&c), 5, &mut scroll2, &mut follow2, &mut flash2, 7,
    ));
    assert!(sel.is_none());
}

// ── selection + yank outcomes ───────────────────────────────────────────────

#[test]
fn v_and_space_toggle_the_anchor() {
    let kb = keybindings();
    let c = cache(&["a", "b"], 40);
    let mut sel = Some(CopySel::entry(1));
    let (mut scroll, mut follow) = (0u32, false);
    handle_key(&plain('v'), &mut sel, &kb, Some(&c), 5, &mut scroll, &mut follow);
    assert_eq!(sel.as_ref().unwrap().anchor, Some(1));
    // Second press drops the selection.
    handle_key(&plain('v'), &mut sel, &kb, Some(&c), 5, &mut scroll, &mut follow);
    assert_eq!(sel.as_ref().unwrap().anchor, None);
    // Space behaves identically.
    handle_key(&key(KeyCode::Char(' ')), &mut sel, &kb, Some(&c), 5, &mut scroll, &mut follow);
    assert_eq!(sel.as_ref().unwrap().anchor, Some(1));
}

#[test]
fn y_yanks_and_stays_enter_yanks_and_exits() {
    let kb = keybindings();
    let c = cache(&["a", "b"], 40);
    let (mut scroll, mut follow) = (0u32, false);
    let mut sel = Some(CopySel::entry(0));
    assert_eq!(
        handle_key(&plain('y'), &mut sel, &kb, Some(&c), 5, &mut scroll, &mut follow),
        CopyOutcome::Yank
    );
    assert!(sel.is_some(), "y keeps the mode open");
    assert_eq!(
        handle_key(&key(KeyCode::Enter), &mut sel, &kb, Some(&c), 5, &mut scroll, &mut follow),
        CopyOutcome::YankExit
    );
}

#[cfg(test)]
#[path = "copy_select_move_tests.rs"]
mod movement;
