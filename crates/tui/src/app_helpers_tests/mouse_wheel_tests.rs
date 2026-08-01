//! Wheel tests for `handle_mouse` over the queue/steer panel: wheel-up looks
//! at older pending entries, wheel-down returns toward the newest (offset 0 =
//! pinned), and body scrolling is untouched. Split from `mouse_tests.rs` to
//! keep files within the iteration caps.

use super::mouse_helpers::*;
use crate::app_helpers::*;
use ratatui::layout::Rect;

fn wheel_event(kind: MouseEventKind, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column: 40,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

/// Wheel-up over the queue/steer panel scrolls the panel toward older entries
/// and must NOT touch the body scroll/follow state.
#[tokio::test]
async fn wheel_up_in_queue_panel_scrolls_panel_only() {
    let mut chat = ChatView::default();
    let body = Rect::new(0, 4, 80, 12);
    let mut hits = empty_hits(body);
    hits.queue_panel = Some(Rect::new(0, 0, 80, 3));
    hits.queue_total = 6;
    let mut scroll = 10u32;
    let mut follow = false;
    let mut queue_scroll = 0u32;
    let mut selection: Option<SelRange> = None;
    let mut subagent_focus: Option<usize> = None;
    let mut subagent_sys = 0u64;
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let store = StubStore;
    let mut copy_msg: Option<String> = None;
    let mut last_click: Option<Instant> = None;
    let mut dbl_click = false;

    handle_mouse(
        wheel_event(MouseEventKind::ScrollUp, 1),
        &hits,
        &mut scroll,
        &mut follow,
        &mut selection,
        &mut chat,
        &mut subagent_focus,
        &mut subagent_sys,
        Path::new("."),
        &mut queue_items,
        "s",
        &store,
        &mut copy_msg,
        &mut last_click,
        &mut dbl_click,
        &mut queue_scroll,
    )
    .await;

    assert_eq!(
        queue_scroll, 1,
        "wheel-up over panel looks at older entries"
    );
    assert_eq!(scroll, 10, "body scroll untouched");
    assert!(!follow, "body follow untouched");
}

/// Wheel-down over the queue/steer panel moves back toward the newest entries
/// and floors at 0 (pinned to newest).
#[tokio::test]
async fn wheel_down_in_queue_panel_returns_toward_newest() {
    let mut chat = ChatView::default();
    let body = Rect::new(0, 4, 80, 12);
    let mut hits = empty_hits(body);
    hits.queue_panel = Some(Rect::new(0, 0, 80, 3));
    hits.queue_total = 6;
    let mut scroll = 10u32;
    let mut follow = true;
    let mut queue_scroll = 2u32;
    let mut selection: Option<SelRange> = None;
    let mut subagent_focus: Option<usize> = None;
    let mut subagent_sys = 0u64;
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let store = StubStore;
    let mut copy_msg: Option<String> = None;
    let mut last_click: Option<Instant> = None;
    let mut dbl_click = false;

    handle_mouse(
        wheel_event(MouseEventKind::ScrollDown, 1),
        &hits,
        &mut scroll,
        &mut follow,
        &mut selection,
        &mut chat,
        &mut subagent_focus,
        &mut subagent_sys,
        Path::new("."),
        &mut queue_items,
        "s",
        &store,
        &mut copy_msg,
        &mut last_click,
        &mut dbl_click,
        &mut queue_scroll,
    )
    .await;
    assert_eq!(queue_scroll, 1, "one notch toward newest");

    // A second wheel-down crosses the floor 1 -> 0 — never negative.
    handle_mouse(
        wheel_event(MouseEventKind::ScrollDown, 1),
        &hits,
        &mut scroll,
        &mut follow,
        &mut selection,
        &mut chat,
        &mut subagent_focus,
        &mut subagent_sys,
        Path::new("."),
        &mut queue_items,
        "s",
        &store,
        &mut copy_msg,
        &mut last_click,
        &mut dbl_click,
        &mut queue_scroll,
    )
    .await;
    assert_eq!(queue_scroll, 0, "pinned to newest floors at zero");
}

/// A wheel over the body (outside the queue-panel rect) keeps the existing
/// body semantics and leaves the panel offset untouched — no regression.
#[tokio::test]
async fn wheel_outside_queue_panel_scrolls_body() {
    let mut chat = ChatView::default();
    let body = Rect::new(0, 4, 80, 12);
    let mut hits = empty_hits(body);
    hits.queue_panel = Some(Rect::new(0, 0, 80, 3));
    hits.queue_total = 6;
    hits.total_rows = 100;
    let mut scroll = 10u32;
    let mut follow = false;
    let mut queue_scroll = 1u32;
    let mut selection: Option<SelRange> = None;
    let mut subagent_focus: Option<usize> = None;
    let mut subagent_sys = 0u64;
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let store = StubStore;
    let mut copy_msg: Option<String> = None;
    let mut last_click: Option<Instant> = None;
    let mut dbl_click = false;

    // row 6 is inside the body rect (rows 4..16), below the panel (rows 0..3).
    handle_mouse(
        wheel_event(MouseEventKind::ScrollDown, 6),
        &hits,
        &mut scroll,
        &mut follow,
        &mut selection,
        &mut chat,
        &mut subagent_focus,
        &mut subagent_sys,
        Path::new("."),
        &mut queue_items,
        "s",
        &store,
        &mut copy_msg,
        &mut last_click,
        &mut dbl_click,
        &mut queue_scroll,
    )
    .await;

    assert_eq!(scroll, 13, "body scroll advanced by one notch");
    assert_eq!(queue_scroll, 1, "panel offset untouched");
}

/// Without a queue-panel hit rect (panel hidden, e.g. plan mode), the wheel
/// keeps scrolling the body — stale `queue_scroll` stays put and is clamped
/// by the renderer.
#[tokio::test]
async fn wheel_with_no_queue_panel_keeps_body_behavior() {
    let mut chat = ChatView::default();
    let body = Rect::new(0, 0, 80, 12);
    let mut hits = empty_hits(body);
    hits.total_rows = 100;
    let mut scroll = 10u32;
    let mut follow = false;
    let mut queue_scroll = 0u32;
    let mut selection: Option<SelRange> = None;
    let mut subagent_focus: Option<usize> = None;
    let mut subagent_sys = 0u64;
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let store = StubStore;
    let mut copy_msg: Option<String> = None;
    let mut last_click: Option<Instant> = None;
    let mut dbl_click = false;

    handle_mouse(
        wheel_event(MouseEventKind::ScrollUp, 6),
        &hits,
        &mut scroll,
        &mut follow,
        &mut selection,
        &mut chat,
        &mut subagent_focus,
        &mut subagent_sys,
        Path::new("."),
        &mut queue_items,
        "s",
        &store,
        &mut copy_msg,
        &mut last_click,
        &mut dbl_click,
        &mut queue_scroll,
    )
    .await;

    assert_eq!(scroll, 2, "wheel-up over body still scrolls the body");
    assert_eq!(queue_scroll, 0, "no panel → no queue scroll");
}
