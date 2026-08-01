//! Tests for clipboard-copy mouse interactions: Shift+drag and Shift+click
//! select through the app's own pipeline and copy on release, alongside the
//! plain multi-line drag selection path.

use crate::app_helpers::*;
use ratatui::layout::Rect;
use super::mouse_helpers::*;

// ── Shift+drag selection + copy ────────────────────────────────────

#[tokio::test]
async fn shift_drag_starts_selection() {
    let mut chat = view_from_lines(&["alpha", "beta", "gamma"]);
    let body = Rect::new(0, 0, 80, 12);
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
    let mut queue_scroll: u32 = 0;

    // Shift+Down must anchor a selection exactly like a normal drag.
    // Screen rows 1..3 map to content rows 0..2 (inner_y = 1) — the three
    // view lines "alpha"/"beta"/"gamma".
    let shift_down = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 10,
        row: 1,
        modifiers: KeyModifiers::SHIFT,
    };
    handle_mouse(
        shift_down, &hits, &mut scroll, &mut follow, &mut selection, &mut chat,
        &mut subagent_focus, &mut parent_scroll, &mut parent_follow,
        &mut subagent_sys, Path::new("."), &mut queue_items, "s", &store,
        &mut copy_msg, &mut last_click, &mut dbl_click,
            &mut queue_scroll,
    )
    .await;
    assert_eq!(selection, Some((0, 0)), "Shift+Down must anchor a selection");

    // Shift+Drag must extend it.
    let shift_drag = MouseEvent {
        kind: MouseEventKind::Drag(MouseButton::Left),
        column: 10,
        row: 3,
        modifiers: KeyModifiers::SHIFT,
    };
    handle_mouse(
        shift_drag, &hits, &mut scroll, &mut follow, &mut selection, &mut chat,
        &mut subagent_focus, &mut parent_scroll, &mut parent_follow,
        &mut subagent_sys, Path::new("."), &mut queue_items, "s", &store,
        &mut copy_msg, &mut last_click, &mut dbl_click,
            &mut queue_scroll,
    )
    .await;
    assert_eq!(selection, Some((0, 2)), "Shift+Drag must extend the selection");
}

#[tokio::test]
async fn shift_drag_copies_on_release() {
    let mut chat = view_from_lines(&["alpha", "beta", "gamma"]);
    let body = Rect::new(0, 0, 80, 12);
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
    let mut queue_scroll: u32 = 0;

    // Shift+Down + Shift+Drag + Shift+Up: same copy pipeline as a normal drag.
    // Screen rows 1..3 select content rows 0..2 ("alpha".."gamma").
    for (kind, row) in [
        (MouseEventKind::Down(MouseButton::Left), 1u16),
        (MouseEventKind::Drag(MouseButton::Left), 3u16),
    ] {
        let ev = MouseEvent {
            kind,
            column: 10,
            row,
            modifiers: KeyModifiers::SHIFT,
        };
        handle_mouse(
            ev, &hits, &mut scroll, &mut follow, &mut selection, &mut chat,
            &mut subagent_focus, &mut parent_scroll, &mut parent_follow,
            &mut subagent_sys, Path::new("."), &mut queue_items, "s", &store,
            &mut copy_msg, &mut last_click, &mut dbl_click,
            &mut queue_scroll,
        )
        .await;
    }
    assert_eq!(selection, Some((0, 2)), "Shift+drag must track the selection");

    let up = MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        column: 10,
        row: 3,
        modifiers: KeyModifiers::SHIFT,
    };
    handle_mouse(
        up, &hits, &mut scroll, &mut follow, &mut selection, &mut chat,
        &mut subagent_focus, &mut parent_scroll, &mut parent_follow,
        &mut subagent_sys, Path::new("."), &mut queue_items, "s", &store,
        &mut copy_msg, &mut last_click, &mut dbl_click,
            &mut queue_scroll,
    )
    .await;
    assert!(copy_msg.is_some(), "Shift+drag release must copy");
    assert!(
        !copy_msg.as_deref().is_some_and(|m| m.contains("Nothing to copy")),
        "real Shift+drag selection should copy text, got: {copy_msg:?}"
    );
    assert!(selection.is_none(), "selection cleared after release");
}

#[tokio::test]
async fn shift_click_copies_single_line() {
    let mut chat = view_from_lines(&["alpha", "beta", "gamma"]);
    let body = Rect::new(0, 0, 80, 12);
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
    let mut queue_scroll: u32 = 0;

    // Shift+Down then Shift+Up with no drag: single-line copy (force path).
    // Screen row 2 = content row 1 ("beta").
    let shift_down = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 10,
        row: 2,
        modifiers: KeyModifiers::SHIFT,
    };
    handle_mouse(
        shift_down, &hits, &mut scroll, &mut follow, &mut selection, &mut chat,
        &mut subagent_focus, &mut parent_scroll, &mut parent_follow,
        &mut subagent_sys, Path::new("."), &mut queue_items, "s", &store,
        &mut copy_msg, &mut last_click, &mut dbl_click,
            &mut queue_scroll,
    )
    .await;

    let shift_up = MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        column: 10,
        row: 2,
        modifiers: KeyModifiers::SHIFT,
    };
    handle_mouse(
        shift_up, &hits, &mut scroll, &mut follow, &mut selection, &mut chat,
        &mut subagent_focus, &mut parent_scroll, &mut parent_follow,
        &mut subagent_sys, Path::new("."), &mut queue_items, "s", &store,
        &mut copy_msg, &mut last_click, &mut dbl_click,
            &mut queue_scroll,
    )
    .await;
    assert!(copy_msg.is_some(), "Shift+click must copy the line");
    assert!(
        !copy_msg.as_deref().is_some_and(|m| m.contains("Nothing to copy")),
        "Shift+click on a real line should copy, got: {copy_msg:?}"
    );
    assert!(selection.is_none(), "selection cleared after release");
}

// ── Multi-line drag selection ──────────────────────────────────────

#[tokio::test]
async fn multi_line_drag_copies_on_release() {
    let lines: Vec<&str> = (0..20).map(|_| "content line").collect();
    let mut chat = view_from_lines(&lines);
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
    let mut queue_scroll: u32 = 0;

    // Down at row 5, drag to row 7, then release.
    let down = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 10,
        row: 5,
        modifiers: KeyModifiers::NONE,
    };
    handle_mouse(
        down, &hits, &mut scroll, &mut follow, &mut selection, &mut chat,
        &mut subagent_focus, &mut parent_scroll, &mut parent_follow,
        &mut subagent_sys, Path::new("."), &mut queue_items, "s", &store,
        &mut copy_msg, &mut last_click, &mut dbl_click,
            &mut queue_scroll,
    )
    .await;

    let drag = MouseEvent {
        kind: MouseEventKind::Drag(MouseButton::Left),
        column: 10,
        row: 7,
        modifiers: KeyModifiers::NONE,
    };
    handle_mouse(
        drag, &hits, &mut scroll, &mut follow, &mut selection, &mut chat,
        &mut subagent_focus, &mut parent_scroll, &mut parent_follow,
        &mut subagent_sys, Path::new("."), &mut queue_items, "s", &store,
        &mut copy_msg, &mut last_click, &mut dbl_click,
            &mut queue_scroll,
    )
    .await;

    let up = MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        column: 10,
        row: 7,
        modifiers: KeyModifiers::NONE,
    };
    handle_mouse(
        up, &hits, &mut scroll, &mut follow, &mut selection, &mut chat,
        &mut subagent_focus, &mut parent_scroll, &mut parent_follow,
        &mut subagent_sys, Path::new("."), &mut queue_items, "s", &store,
        &mut copy_msg, &mut last_click, &mut dbl_click,
            &mut queue_scroll,
    )
    .await;

    assert!(copy_msg.is_some(), "drag should copy on release");
    assert!(selection.is_none(), "selection cleared after release");
}
