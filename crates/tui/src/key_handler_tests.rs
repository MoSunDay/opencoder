//! Unit tests for `handle_key` / `apply_scroll`: scroll paging, disabled-input
//! gating, clipboard (Ctrl+V), and agent-switch tab behavior. Extracted from
//! `key_handler.rs` to keep it under the 800-line file-size cap.

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
    let mut show_help = false;
    let mut scroll = 0u32;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut skill_menu: Option<SkillMenu> = None;

    let action = handle_key(
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
        &mut input,
        &mut cursor,
        &history,
        &mut hist_idx,
        false,
        "act",
        &mut show_help,
        &mut scroll,
        &mut follow,
        &mut last_esc,
        &mut skill_menu,
        80,
        2,
        false,
        true,
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
    let mut show_help = false;
    let mut scroll = 0u32;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut skill_menu: Option<SkillMenu> = None;

    let action = handle_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &mut input,
        &mut cursor,
        &history,
        &mut hist_idx,
        false,
        "act",
        &mut show_help,
        &mut scroll,
        &mut follow,
        &mut last_esc,
        &mut skill_menu,
        80,
        2,
        false,
        true,
    );
    assert!(matches!(action, KeyAction::None));
}

#[test]
fn handle_key_disabled_allows_scroll() {
    let mut input = String::new();
    let mut cursor = 0usize;
    let history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;
    let mut show_help = false;
    let mut scroll = 50u32;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut skill_menu: Option<SkillMenu> = None;

    let action = handle_key(
        KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE),
        &mut input,
        &mut cursor,
        &history,
        &mut hist_idx,
        false,
        "act",
        &mut show_help,
        &mut scroll,
        &mut follow,
        &mut last_esc,
        &mut skill_menu,
        80,
        2,
        false,
        true,
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
    let mut show_help = false;
    let mut scroll = 0u32;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut skill_menu: Option<SkillMenu> = None;

    let action = handle_key(
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
        &mut input,
        &mut cursor,
        &history,
        &mut hist_idx,
        false,
        "act",
        &mut show_help,
        &mut scroll,
        &mut follow,
        &mut last_esc,
        &mut skill_menu,
        80,
        2,
        false,
        true,
    );
    assert!(matches!(action, KeyAction::Quit));
}

#[test]
fn ctrl_v_returns_clip() {
    let mut input = String::new();
    let mut cursor = 0usize;
    let history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;
    let mut show_help = false;
    let mut scroll = 0u32;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut skill_menu: Option<SkillMenu> = None;

    let action = handle_key(
        KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL),
        &mut input,
        &mut cursor,
        &history,
        &mut hist_idx,
        false,
        "act",
        &mut show_help,
        &mut scroll,
        &mut follow,
        &mut last_esc,
        &mut skill_menu,
        80,
        2,
        false,
        false,
    );
    assert!(matches!(action, KeyAction::Clip));
}

#[test]
fn handle_key_disabled_blocks_alt_tab() {
    let mut input = String::new();
    let mut cursor = 0usize;
    let history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;
    let mut show_help = false;
    let mut scroll = 0u32;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut skill_menu: Option<SkillMenu> = None;

    // Alt+Tab must be blocked when input is disabled (subagent-focus
    // view) so the parent agent is not switched prematurely.
    let action = handle_key(
        KeyEvent::new(KeyCode::Tab, KeyModifiers::ALT),
        &mut input,
        &mut cursor,
        &history,
        &mut hist_idx,
        false,
        "plan",
        &mut show_help,
        &mut scroll,
        &mut follow,
        &mut last_esc,
        &mut skill_menu,
        80,
        2,
        false,
        true,
    );
    assert!(matches!(action, KeyAction::None));

    // Alt+BackTab variant.
    let action = handle_key(
        KeyEvent::new(KeyCode::BackTab, KeyModifiers::ALT),
        &mut input,
        &mut cursor,
        &history,
        &mut hist_idx,
        false,
        "plan",
        &mut show_help,
        &mut scroll,
        &mut follow,
        &mut last_esc,
        &mut skill_menu,
        80,
        2,
        false,
        true,
    );
    assert!(matches!(action, KeyAction::None));
}

#[test]
fn handle_key_disabled_blocks_ctrl_shift_tab() {
    let mut input = String::new();
    let mut cursor = 0usize;
    let history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;
    let mut show_help = false;
    let mut scroll = 0u32;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut skill_menu: Option<SkillMenu> = None;

    // Ctrl+Shift+Tab (BackTab+CONTROL) must be blocked when input is
    // disabled so the parent agent is not switched prematurely.
    let action = handle_key(
        KeyEvent::new(KeyCode::BackTab, KeyModifiers::CONTROL),
        &mut input,
        &mut cursor,
        &history,
        &mut hist_idx,
        false,
        "plan",
        &mut show_help,
        &mut scroll,
        &mut follow,
        &mut last_esc,
        &mut skill_menu,
        80,
        2,
        false,
        true,
    );
    assert!(matches!(action, KeyAction::None));

    // kitty: Tab+CONTROL+SHIFT.
    let action = handle_key(
        KeyEvent::new(KeyCode::Tab, KeyModifiers::CONTROL | KeyModifiers::SHIFT),
        &mut input,
        &mut cursor,
        &history,
        &mut hist_idx,
        false,
        "plan",
        &mut show_help,
        &mut scroll,
        &mut follow,
        &mut last_esc,
        &mut skill_menu,
        80,
        2,
        false,
        true,
    );
    assert!(matches!(action, KeyAction::None));
}

