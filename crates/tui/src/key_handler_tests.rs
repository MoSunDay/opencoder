//! Unit tests for `handle_key` / `apply_scroll`: scroll paging, disabled-input
//! gating, clipboard (Ctrl+V), and the Shift+Tab clear-context submit.
//! Extracted from `key_handler.rs` to keep it under the 800-line cap.

use super::*;

#[test]
fn apply_scroll_page_up() {
    let mut scroll = 50u32;
    let mut follow = true;
    let k = KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE);
    assert!(apply_scroll(&k, &mut scroll, &mut follow));
    assert_eq!(scroll, 30);
    assert!(!follow);
}

#[test]
fn apply_scroll_page_down() {
    let mut scroll = 50u32;
    let mut follow = false;
    let k = KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE);
    assert!(apply_scroll(&k, &mut scroll, &mut follow));
    assert!(follow);
}

#[test]
fn apply_scroll_char_not_consumed() {
    let mut scroll = 50u32;
    let mut follow = true;
    let k = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
    assert!(!apply_scroll(&k, &mut scroll, &mut follow));
    assert_eq!(scroll, 50);
    assert!(follow);
}

#[test]
fn handle_key_disabled_blocks_char() {
    let mut input = String::new();
    let mut cursor = 0usize;
    let history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;
    let mut scroll = 0u32;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut skill_menu: Option<SkillMenu> = None;
    let mut undo_state = crate::undo::init("", 0);
    let mut queue_scroll: u32 = 0;

    let action = handle_key(
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
        &mut input,
        &mut cursor,
        &history,
        &mut hist_idx,
        false,
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
        true,
        &mut undo_state,
        &mut queue_scroll,
        &mut None,
    );
    assert!(matches!(action, KeyAction::None));
    assert!(input.is_empty());
}

#[test]
fn handle_key_disabled_blocks_enter() {
    let mut input = String::new();
    let mut cursor = 0usize;
    let history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;
    let mut scroll = 0u32;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut skill_menu: Option<SkillMenu> = None;
    let mut undo_state = crate::undo::init("", 0);
    let mut queue_scroll: u32 = 0;

    let action = handle_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
        &mut input,
        &mut cursor,
        &history,
        &mut hist_idx,
        false,
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
        true,
        &mut undo_state,
        &mut queue_scroll,
        &mut None,
    );
    assert!(matches!(action, KeyAction::None));
}

#[test]
fn handle_key_disabled_allows_scroll() {
    let mut input = String::new();
    let mut cursor = 0usize;
    let history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;
    let mut scroll = 50u32;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut skill_menu: Option<SkillMenu> = None;
    let mut undo_state = crate::undo::init("", 0);
    let mut queue_scroll: u32 = 0;

    let action = handle_key(
        KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE),
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
        &mut input,
        &mut cursor,
        &history,
        &mut hist_idx,
        false,
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
        true,
        &mut undo_state,
        &mut queue_scroll,
        &mut None,
    );
    assert!(matches!(action, KeyAction::None));
    assert_eq!(scroll, 30);
    assert!(!follow);
}

#[test]
fn handle_key_disabled_allows_quit() {
    let mut input = String::new();
    let mut cursor = 0usize;
    let history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;
    let mut scroll = 0u32;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut skill_menu: Option<SkillMenu> = None;
    let mut undo_state = crate::undo::init("", 0);
    let mut queue_scroll: u32 = 0;

    let action = handle_key(
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
        &mut input,
        &mut cursor,
        &history,
        &mut hist_idx,
        false,
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
        true,
        &mut undo_state,
        &mut queue_scroll,
        &mut None,
    );
    assert!(matches!(action, KeyAction::Quit));
}

#[path = "key_handler_tests/bash.rs"]
mod bash;
#[path = "key_handler_tests/editing.rs"]
mod editing;
