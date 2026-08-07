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
    let mut undo_state = crate::undo::init("", 0);
    let mut help_scroll: u16 = 0;
    let mut queue_scroll: u32 = 0;

    let action = handle_key(
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
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
        &mut undo_state,
        &mut help_scroll,
        &mut queue_scroll,
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
    let mut undo_state = crate::undo::init("", 0);
    let mut help_scroll: u16 = 0;
    let mut queue_scroll: u32 = 0;

    let action = handle_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
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
        &mut undo_state,
        &mut help_scroll,
        &mut queue_scroll,
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
    let mut undo_state = crate::undo::init("", 0);
    let mut help_scroll: u16 = 0;
    let mut queue_scroll: u32 = 0;

    let action = handle_key(
        KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE),
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
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
        &mut undo_state,
        &mut help_scroll,
        &mut queue_scroll,
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
    let mut undo_state = crate::undo::init("", 0);
    let mut help_scroll: u16 = 0;
    let mut queue_scroll: u32 = 0;

    let action = handle_key(
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
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
        &mut undo_state,
        &mut help_scroll,
        &mut queue_scroll,
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
    let mut undo_state = crate::undo::init("", 0);
    let mut help_scroll: u16 = 0;
    let mut queue_scroll: u32 = 0;

    let action = handle_key(
        KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL),
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
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
        &mut undo_state,
        &mut help_scroll,
        &mut queue_scroll,
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
    let mut undo_state = crate::undo::init("", 0);
    let mut help_scroll: u16 = 0;
    let mut queue_scroll: u32 = 0;

    // Alt+Tab must be blocked when input is disabled (subagent-focus
    // view) so the parent agent is not switched prematurely.
    let action = handle_key(
        KeyEvent::new(KeyCode::Tab, KeyModifiers::ALT),
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
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
        &mut undo_state,
        &mut help_scroll,
        &mut queue_scroll,
    );
    assert!(matches!(action, KeyAction::None));

    // Alt+BackTab variant.
    let action = handle_key(
        KeyEvent::new(KeyCode::BackTab, KeyModifiers::ALT),
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
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
        &mut undo_state,
        &mut help_scroll,
        &mut queue_scroll,
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
    let mut undo_state = crate::undo::init("", 0);
    let mut help_scroll: u16 = 0;
    let mut queue_scroll: u32 = 0;

    // Ctrl+Shift+Tab (BackTab+CONTROL) must be blocked when input is
    // disabled so the parent agent is not switched prematurely.
    let action = handle_key(
        KeyEvent::new(KeyCode::BackTab, KeyModifiers::CONTROL),
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
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
        &mut undo_state,
        &mut help_scroll,
        &mut queue_scroll,
    );
    assert!(matches!(action, KeyAction::None));

    // kitty: Tab+CONTROL+SHIFT.
    let action = handle_key(
        KeyEvent::new(KeyCode::Tab, KeyModifiers::CONTROL | KeyModifiers::SHIFT),
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
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
        &mut undo_state,
        &mut help_scroll,
        &mut queue_scroll,
    );
    assert!(matches!(action, KeyAction::None));
}

// ---------------------------------------------------------------------------
// Undo/redo (Ctrl+Z / Ctrl+Y)
// ---------------------------------------------------------------------------

#[test]
fn undo_restores_previous_text() {
    let mut input = String::new();
    let mut cursor = 0usize;
    let history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;
    let mut show_help = false;
    let mut scroll = 0u32;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut skill_menu: Option<SkillMenu> = None;
    let mut undo_state = crate::undo::init("", 0);
    let mut help_scroll: u16 = 0;
    let mut queue_scroll: u32 = 0;

    // Type "hi"
    for ch in ['h', 'i'] {
        handle_key(
            KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
            &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
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
            &mut undo_state,
            &mut help_scroll,
            &mut queue_scroll,
        );
    }
    assert_eq!(input, "hi");

    // Ctrl+Z undoes both chars (collapsed) back to ""
    handle_key(
        KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL),
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
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
        &mut undo_state,
        &mut help_scroll,
        &mut queue_scroll,
    );
    assert_eq!(input, "");

    // Ctrl+Y redoes
    handle_key(
        KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL),
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
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
        &mut undo_state,
        &mut help_scroll,
        &mut queue_scroll,
    );
    assert_eq!(input, "hi");
}

#[test]
fn undo_after_backspace() {
    let mut input = "hello".to_string();
    let mut cursor = 5usize;
    let history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;
    let mut show_help = false;
    let mut scroll = 0u32;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut skill_menu: Option<SkillMenu> = None;
    let mut undo_state = crate::undo::init("hello", 5);
    let mut help_scroll: u16 = 0;
    let mut queue_scroll: u32 = 0;

    // Backspace
    handle_key(
        KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
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
        &mut undo_state,
        &mut help_scroll,
        &mut queue_scroll,
    );
    assert_eq!(input, "hell");

    // Undo
    handle_key(
        KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL),
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
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
        &mut undo_state,
        &mut help_scroll,
        &mut queue_scroll,
    );
    assert_eq!(input, "hello");
}

// ---------------------------------------------------------------------------
// Help popup scrolling
// ---------------------------------------------------------------------------

#[test]
fn help_open_down_arrow_increments_scroll() {
    let mut input = String::new();
    let mut cursor = 0usize;
    let history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;
    let mut show_help = true;
    let mut scroll = 0u32;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut skill_menu: Option<SkillMenu> = None;
    let mut undo_state = crate::undo::init("", 0);
    let mut help_scroll: u16 = 0;
    let mut queue_scroll: u32 = 0;

    let action = handle_key(
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
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
        &mut undo_state,
        &mut help_scroll,
        &mut queue_scroll,
    );
    assert!(matches!(action, KeyAction::None));
    assert_eq!(help_scroll, 1);
}

#[test]
fn help_open_page_down_jumps_scroll() {
    let mut input = String::new();
    let mut cursor = 0usize;
    let history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;
    let mut show_help = true;
    let mut scroll = 0u32;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut skill_menu: Option<SkillMenu> = None;
    let mut undo_state = crate::undo::init("", 0);
    let mut help_scroll: u16 = 0;
    let mut queue_scroll: u32 = 0;

    let action = handle_key(
        KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE),
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
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
        &mut undo_state,
        &mut help_scroll,
        &mut queue_scroll,
    );
    assert!(matches!(action, KeyAction::None));
    assert_eq!(help_scroll, 0); // 5 - 10 saturated to 0
}

// ---------------------------------------------------------------------------
// History navigation: Up/Down with cursor_row_col boundary detection
// ---------------------------------------------------------------------------

#[test]
fn up_arrow_browses_history_when_single_row() {
    let mut input = "current".to_string();
    let mut cursor = 7usize;
    let history = vec!["older".to_string()];
    let mut hist_idx: Option<usize> = None;
    let mut show_help = false;
    let mut scroll = 0u32;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut skill_menu: Option<SkillMenu> = None;
    let mut undo_state = crate::undo::init("current", 7);
    let mut help_scroll: u16 = 0;
    let mut queue_scroll: u32 = 0;

    // Single-row input (7 chars < row_w=78), so Up browses history.
    handle_key(
        KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
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
        &mut undo_state,
        &mut help_scroll,
        &mut queue_scroll,
    );
    assert_eq!(input, "older");
    assert_eq!(hist_idx, Some(0));
}

#[test]
fn up_arrow_moves_cursor_when_multi_row() {
    // Long input that wraps to multiple rows.
    let input_text = "abcdefghij".repeat(10); // 100 chars
    let mut input = input_text.clone();
    let mut cursor = 80usize; // row 1 (row_w=78)
    let history = vec!["older".to_string()];
    let mut hist_idx: Option<usize> = None;
    let mut show_help = false;
    let mut scroll = 0u32;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut skill_menu: Option<SkillMenu> = None;
    let mut undo_state = crate::undo::init(&input_text, 80);
    let mut help_scroll: u16 = 0;
    let mut queue_scroll: u32 = 0;

    // Multi-row: cursor at row > 0, so Up moves cursor up (not history).
    let cursor_before = cursor;
    handle_key(
        KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
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
        &mut undo_state,
        &mut help_scroll,
        &mut queue_scroll,
    );
    // Cursor moved up, input unchanged, history not browsed.
    assert!(cursor < cursor_before, "cursor should move up");
    assert_eq!(input, input_text);
    assert_eq!(hist_idx, None);
}

#[test]
fn handle_key_alt_char_is_dropped_not_inserted() {
    // Esc+char (tmux escape-time merges into Alt+char; some terminals deliver
    // Alt as an ESC prefix) must never reach the input box: unhandled Alt
    // combos are dropped, not typed as garbage like `[D` / `[A`.
    let mut input = String::new();
    let mut cursor = 0usize;
    let history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;
    let mut show_help = false;
    let mut scroll = 0u32;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut skill_menu: Option<SkillMenu> = None;
    let mut undo_state = crate::undo::init("", 0);
    let mut help_scroll: u16 = 0;
    let mut queue_scroll: u32 = 0;

    let action = handle_key(
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT),
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
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
        &mut undo_state,
        &mut help_scroll,
        &mut queue_scroll,
    );
    assert!(matches!(action, KeyAction::None));
    assert!(input.is_empty());
    assert_eq!(cursor, 0);
}

#[test]
fn handle_key_alt_f_still_moves_word() {
    // Alt+F (readline forward-word) is an explicit binding and must survive
    // the Alt+Char guard (it is handled before the Char fallback).
    let mut input = "hello".to_string();
    let mut cursor = 0usize;
    let history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;
    let mut show_help = false;
    let mut scroll = 0u32;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut skill_menu: Option<SkillMenu> = None;
    let mut undo_state = crate::undo::init("hello", 0);
    let mut help_scroll: u16 = 0;
    let mut queue_scroll: u32 = 0;

    let action = handle_key(
        KeyEvent::new(KeyCode::Char('f'), KeyModifiers::ALT),
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
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
        &mut undo_state,
        &mut help_scroll,
        &mut queue_scroll,
    );
    assert!(matches!(action, KeyAction::None));
    assert_eq!(input, "hello");
    assert_eq!(
        cursor, 5,
        "Alt+F must still move the cursor to the word end"
    );
}
