//! Unit tests for queue/steer panel scroll keys (Shift+PageUp / Shift+PageDown)
//! and the regression guard that plain PageUp keeps body semantics. Split from
//! `key_handler_tests.rs` to keep that file within the 800-line iteration cap.

use super::*;

#[test]
fn shift_page_up_scrolls_queue_panel_not_body() {
    let mut input = String::new();
    let mut cursor = 0usize;
    let history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;
    let mut show_help = false;
    let mut scroll = 50u32;
    let mut follow = false;
    let mut last_esc: Option<Instant> = None;
    let mut skill_menu: Option<SkillMenu> = None;
    let mut undo_state = crate::undo::init("", 0);
    let mut help_scroll: u16 = 0;
    let mut queue_scroll: u32 = 2;

    let action = handle_key(
        KeyEvent::new(KeyCode::PageUp, KeyModifiers::SHIFT),
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
    assert_eq!(queue_scroll, 1, "Shift+PageUp looks at older entries (top)");
    assert_eq!(scroll, 50, "body scroll untouched");
    assert!(!follow, "body follow untouched");
}

#[test]
fn shift_page_down_advances_toward_newest() {
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
    let mut queue_scroll: u32 = 3;

    let action = handle_key(
        KeyEvent::new(KeyCode::PageDown, KeyModifiers::SHIFT),
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
    assert_eq!(queue_scroll, 4, "Shift+PageDown moves toward newer entries (bottom)");
    assert_eq!(scroll, 50, "body scroll untouched");
    assert!(follow, "body follow untouched");
}

#[test]
fn shift_page_up_floors_at_zero() {
    // The floor crossing (1 -> 0) must clamp, and a further press must stay
    // pinned at 0 — never negative.
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
    let mut queue_scroll: u32 = 1;

    for _ in 0..2 {
        let action = handle_key(
            KeyEvent::new(KeyCode::PageUp, KeyModifiers::SHIFT),
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
    }
    assert_eq!(queue_scroll, 0, "floor at top, pinned at zero");
}

#[test]
fn plain_page_up_still_scrolls_body() {
    // Regression guard: without SHIFT, PageUp must keep body semantics even
    // though queue_scroll exists in the signature.
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
    assert_eq!(scroll, 30, "plain PageUp scrolls the body");
    assert!(!follow);
    assert_eq!(queue_scroll, 0, "queue panel untouched");
}
