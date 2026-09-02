//! Wheel tests for `handle_mouse` over the queue/steer panel: wheel-up looks
//! at older pending entries (toward the top), wheel-down advances toward newer
//! ones (toward the bottom), and body scrolling is untouched. Split from
//! `mouse_tests.rs` to keep files within the iteration caps.

use super::mouse_helpers::*;
use crate::app_helpers::*;
use crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};
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
    let mut queue_scroll = 2u32;
    let mut subagent_focus: Option<usize> = None;
    let mut subagent_sys = 0u64;
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let store = StubStore::default();

    handle_mouse(
        wheel_event(MouseEventKind::ScrollUp, 1),
        &hits,
        &mut scroll,
        &mut follow,
        &mut chat,
        &mut subagent_focus,
        &mut subagent_sys,
        Path::new("."),
        &mut queue_items,
        "s",
        &store,
        &mut queue_scroll,
        &mut vec![], // no pending images
    )
    .await;

    assert_eq!(
        queue_scroll, 1,
        "wheel-up over panel looks at older entries (top)"
    );
    assert_eq!(scroll, 10, "body scroll untouched");
    assert!(!follow, "body follow untouched");
}

/// Wheel-down over the queue/steer panel advances toward the newer (bottom)
/// entries and clamps at `max_scroll` (never overshoots).
#[tokio::test]
async fn wheel_down_advances_toward_newest() {
    let mut chat = ChatView::default();
    let body = Rect::new(0, 4, 80, 12);
    let mut hits = empty_hits(body);
    hits.queue_panel = Some(Rect::new(0, 0, 80, 3));
    hits.queue_total = 6;
    let mut scroll = 10u32;
    let mut follow = true;
    let mut queue_scroll = 1u32;
    let mut subagent_focus: Option<usize> = None;
    let mut subagent_sys = 0u64;
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let store = StubStore::default();

    handle_mouse(
        wheel_event(MouseEventKind::ScrollDown, 1),
        &hits,
        &mut scroll,
        &mut follow,
        &mut chat,
        &mut subagent_focus,
        &mut subagent_sys,
        Path::new("."),
        &mut queue_items,
        "s",
        &store,
        &mut queue_scroll,
        &mut vec![], // no pending images
    )
    .await;
    assert_eq!(queue_scroll, 2, "one notch toward newer entries");

    // A second wheel-down hits the max_scroll (6 - 3 = 3) — never overshoots.
    handle_mouse(
        wheel_event(MouseEventKind::ScrollDown, 1),
        &hits,
        &mut scroll,
        &mut follow,
        &mut chat,
        &mut subagent_focus,
        &mut subagent_sys,
        Path::new("."),
        &mut queue_items,
        "s",
        &store,
        &mut queue_scroll,
        &mut vec![], // no pending images
    )
    .await;
    assert_eq!(queue_scroll, 3, "clamped at max_scroll, pinned to bottom");
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
    let mut subagent_focus: Option<usize> = None;
    let mut subagent_sys = 0u64;
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let store = StubStore::default();

    // row 6 is inside the body rect (rows 4..16), below the panel (rows 0..3).
    handle_mouse(
        wheel_event(MouseEventKind::ScrollDown, 6),
        &hits,
        &mut scroll,
        &mut follow,
        &mut chat,
        &mut subagent_focus,
        &mut subagent_sys,
        Path::new("."),
        &mut queue_items,
        "s",
        &store,
        &mut queue_scroll,
        &mut vec![], // no pending images
    )
    .await;

    assert_eq!(scroll, 13, "body scroll advanced by one notch");
    assert_eq!(queue_scroll, 1, "panel offset untouched");
}

/// Without a queue-panel hit rect (panel hidden, e.g. in the plan editor), the wheel
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
    let mut subagent_focus: Option<usize> = None;
    let mut subagent_sys = 0u64;
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let store = StubStore::default();

    handle_mouse(
        wheel_event(MouseEventKind::ScrollUp, 6),
        &hits,
        &mut scroll,
        &mut follow,
        &mut chat,
        &mut subagent_focus,
        &mut subagent_sys,
        Path::new("."),
        &mut queue_items,
        "s",
        &store,
        &mut queue_scroll,
        &mut vec![], // no pending images
    )
    .await;

    assert_eq!(scroll, 2, "wheel-up over body still scrolls the body");
    assert_eq!(queue_scroll, 0, "no panel → no queue scroll");
}
