//! Tests for the double-click text-selection path: double-clicking a blank
//! line must surface an honest "Nothing to copy" message, plus the pure
//! `is_within_dbl_click_window` timing boundary.

use crate::app_helpers::*;
use ratatui::layout::Rect;
use super::mouse_helpers::*;

// ── Double-click on blank line ─────────────────────────────────────

#[tokio::test]
async fn double_click_blank_line_shows_nothing_to_copy() {
    let mut chat = ChatView::default();
    chat.push_marker(ratatui::text::Line::from("real content".to_string()));
    chat.push_marker(ratatui::text::Line::from("".to_string())); // blank at content row 1
    chat.push_marker(ratatui::text::Line::from("more content".to_string()));

    let body = Rect::new(0, 0, 80, 24);
    let hits = empty_hits(body);
    let store = StubStore;

    let mut scroll = 0u32;
    let mut follow = true;
    let mut selection: Option<SelRange> = None;
    let mut subagent_focus = None;
    let mut parent_scroll = 0u32;
    let mut parent_follow = true;
    let mut subagent_sys = 0u64;
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let mut copy_msg: Option<String> = None;
    let mut last_click: Option<std::time::Instant> = None;
    let mut dbl_click = false;

    // Two quick clicks at screen row 2 -> content row 1 (the blank line).
    for _ in 0..2 {
        let down = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row: 2,
            modifiers: KeyModifiers::NONE,
        };
        handle_mouse(
            down, &hits, &mut scroll, &mut follow, &mut selection, &mut chat,
            &mut subagent_focus, &mut parent_scroll, &mut parent_follow,
            &mut subagent_sys, Path::new("."), &mut queue_items, "s", &store,
            &mut copy_msg, &mut last_click, &mut dbl_click,
        )
        .await;
    }
    assert!(dbl_click, "should detect double-click");

    // Release - blank line yields no text, should show "Nothing to copy".
    let up = MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        column: 10,
        row: 2,
        modifiers: KeyModifiers::NONE,
    };
    handle_mouse(
        up, &hits, &mut scroll, &mut follow, &mut selection, &mut chat,
        &mut subagent_focus, &mut parent_scroll, &mut parent_follow,
        &mut subagent_sys, Path::new("."), &mut queue_items, "s", &store,
        &mut copy_msg, &mut last_click, &mut dbl_click,
    )
    .await;

    assert!(
        copy_msg
            .as_deref()
            .is_some_and(|m| m.contains("Nothing to copy")),
        "blank-line double-click should show 'Nothing to copy', got: {copy_msg:?}"
    );
}

// ── Pure timing function ───────────────────────────────────────────

#[test]
fn dbl_click_window_within_threshold() {
    let now = std::time::Instant::now();
    let prev = now; // same instant -> always within
    assert!(is_within_dbl_click_window(prev, now));
}

#[test]
fn dbl_click_window_beyond_500ms() {
    let now = std::time::Instant::now();
    let prev = now
        .checked_sub(std::time::Duration::from_millis(600))
        .unwrap();
    assert!(!is_within_dbl_click_window(prev, now));
}
