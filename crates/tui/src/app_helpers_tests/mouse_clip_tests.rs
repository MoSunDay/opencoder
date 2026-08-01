//! Tests for clipboard-copy mouse interactions: Shift+drag and Shift+click
//! select through the app's own pipeline and copy on release, alongside the
//! plain multi-line drag selection path.

use super::mouse_helpers::*;
use crate::app_helpers::*;
use opencoder_session::SessionEvent;
use ratatui::layout::Rect;

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
        shift_down,
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
        selection,
        Some((0, 0)),
        "Shift+Down must anchor a selection"
    );

    // Shift+Drag must extend it.
    let shift_drag = MouseEvent {
        kind: MouseEventKind::Drag(MouseButton::Left),
        column: 10,
        row: 3,
        modifiers: KeyModifiers::SHIFT,
    };
    handle_mouse(
        shift_drag,
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
        selection,
        Some((0, 2)),
        "Shift+Drag must extend the selection"
    );
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
            ev,
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
    }
    assert_eq!(
        selection,
        Some((0, 2)),
        "Shift+drag must track the selection"
    );

    let up = MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        column: 10,
        row: 3,
        modifiers: KeyModifiers::SHIFT,
    };
    handle_mouse(
        up,
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
    assert!(copy_msg.is_some(), "Shift+drag release must copy");
    assert!(
        !copy_msg
            .as_deref()
            .is_some_and(|m| m.contains("Nothing to copy")),
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
        shift_down,
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

    let shift_up = MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        column: 10,
        row: 2,
        modifiers: KeyModifiers::SHIFT,
    };
    handle_mouse(
        shift_up,
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
    assert!(copy_msg.is_some(), "Shift+click must copy the line");
    assert!(
        !copy_msg
            .as_deref()
            .is_some_and(|m| m.contains("Nothing to copy")),
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
        down,
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

    let drag = MouseEvent {
        kind: MouseEventKind::Drag(MouseButton::Left),
        column: 10,
        row: 7,
        modifiers: KeyModifiers::NONE,
    };
    handle_mouse(
        drag,
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

    let up = MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        column: 10,
        row: 7,
        modifiers: KeyModifiers::NONE,
    };
    handle_mouse(
        up,
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

    assert!(copy_msg.is_some(), "drag should copy on release");
    assert!(selection.is_none(), "selection cleared after release");
}

// ── Subagent view: copy reads the focused child ────────────────────

// `parent_with_long_subagent` is shared with the scroll/wheel tests via
// `mouse_helpers` (both modules glob-import it).

#[tokio::test]
async fn subagent_view_drag_copies_child_text() {
    let mut chat = parent_with_long_subagent();
    let sub_idx = chat
        .blocks
        .iter()
        .position(|b| matches!(b, crate::chat::ChatBlock::Subagent { .. }))
        .expect("a Subagent block exists");
    let child_rows = match &chat.blocks[sub_idx] {
        crate::chat::ChatBlock::Subagent { view, .. } => view.flatten().len(),
        _ => unreachable!(),
    };
    let parent_rows = chat.flatten().len();

    // Select child rows 10..12 — beyond the parent's whole content. If the
    // copy pipeline wrongly read the parent view, the range finds nothing
    // there and surfaces "Nothing to copy".
    let body = Rect::new(0, 0, 80, 24);
    let hits = empty_hits(body);
    let store = StubStore;

    let mut scroll = 0u32;
    let mut follow = true;
    let mut selection: Option<SelRange> = None;
    let mut subagent_focus = Some(sub_idx);
    let mut subagent_sys = 0u64;
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let mut copy_msg: Option<String> = None;
    let mut last_click: Option<std::time::Instant> = None;
    let mut dbl_click = false;
    let mut queue_scroll: u32 = 0;

    assert!(
        parent_rows < 10,
        "precondition: parent ({parent_rows}) must not reach child rows 10..12"
    );
    assert!(
        child_rows > 12,
        "precondition: child ({child_rows}) must own rows 10..12"
    );

    // Screen rows 11..13 map to child rows 10..12 (inner_y = 1).
    for (kind, row) in [
        (MouseEventKind::Down(MouseButton::Left), 11u16),
        (MouseEventKind::Drag(MouseButton::Left), 13u16),
    ] {
        let ev = MouseEvent {
            kind,
            column: 10,
            row,
            modifiers: KeyModifiers::SHIFT,
        };
        handle_mouse(
            ev,
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
    }
    assert_eq!(
        selection,
        Some((10, 12)),
        "drag must select child rows 10..12"
    );

    let up = MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        column: 10,
        row: 13,
        modifiers: KeyModifiers::SHIFT,
    };
    handle_mouse(
        up,
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
    assert!(copy_msg.is_some(), "release must copy child content");
    assert!(
        !copy_msg
            .as_deref()
            .is_some_and(|m| m.contains("Nothing to copy")),
        "child rows beyond the parent's content must still copy, got: {copy_msg:?}"
    );
    assert!(selection.is_none(), "selection cleared after release");
}

#[tokio::test]
async fn main_view_drag_copies_across_thinking_tool_blocks() {
    // Expanded Thinking (header + 2 lines) followed by an expanded Tool
    // (header + 2 output lines + trailing blank): 7 flattened rows.
    let mut chat = ChatView::default();
    chat.apply(&SessionEvent::ReasoningDelta(
        "think line 1\nthink line 2".into(),
    ));
    chat.toggle_thinking_at(0);
    chat.apply(&SessionEvent::ToolStart {
        id: "t1".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo A"}),
    });
    chat.apply(&SessionEvent::ToolEnd {
        id: "t1".into(),
        name: "bash".into(),
        output: "tool out 1\ntool out 2".into(),
        is_error: false,
        images: Vec::new(),
    });
    chat.toggle_tool_at(1);

    let flattened: Vec<String> = chat
        .flatten()
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
        })
        .collect();
    assert_eq!(flattened.len(), 7, "expanded thinking (3) + tool (4) rows");
    assert!(
        flattened[0].starts_with('\u{1f4ad}'),
        "row 0 = thinking header"
    );
    assert_eq!(flattened[1], "  think line 1");
    assert_eq!(flattened[2], "  think line 2");
    assert!(
        flattened[3].starts_with('\u{25be}'),
        "row 3 = expanded tool header"
    );
    assert_eq!(flattened[4], "  tool out 1");
    assert_eq!(flattened[5], "  tool out 2");
    assert_eq!(flattened[6], "", "row 6 = trailing blank");

    // Drag across the block boundary: rows 0..4 cover the thinking header,
    // both thinking lines, the tool header and the first tool output line.
    let body = Rect::new(0, 0, 80, 24);
    let hits = empty_hits(body);
    let store = StubStore;

    let mut scroll = 0u32;
    let mut follow = true;
    let mut selection: Option<SelRange> = None;
    let mut subagent_focus = None;
    let mut subagent_sys = 0u64;
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let mut copy_msg: Option<String> = None;
    let mut last_click: Option<std::time::Instant> = None;
    let mut dbl_click = false;
    let mut queue_scroll: u32 = 0;

    // Screen rows 1..5 map to content rows 0..4 (inner_y = 1).
    for (kind, row) in [
        (MouseEventKind::Down(MouseButton::Left), 1u16),
        (MouseEventKind::Drag(MouseButton::Left), 5u16),
    ] {
        let ev = MouseEvent {
            kind,
            column: 10,
            row,
            modifiers: KeyModifiers::NONE,
        };
        handle_mouse(
            ev,
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
    }
    assert_eq!(
        selection,
        Some((0, 4)),
        "drag must span thinking + tool rows"
    );

    let up = MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        column: 10,
        row: 5,
        modifiers: KeyModifiers::NONE,
    };
    handle_mouse(
        up,
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
    assert!(copy_msg.is_some(), "cross-block drag must copy on release");
    assert!(
        !copy_msg
            .as_deref()
            .is_some_and(|m| m.contains("Nothing to copy")),
        "flattened rows across blocks must copy, got: {copy_msg:?}"
    );
    assert!(selection.is_none(), "selection cleared after release");
}

#[tokio::test]
async fn subagent_view_shift_click_copies_line() {
    let mut chat = parent_with_long_subagent();
    let sub_idx = chat
        .blocks
        .iter()
        .position(|b| matches!(b, crate::chat::ChatBlock::Subagent { .. }))
        .expect("a Subagent block exists");

    // Shift+click a child row beyond the parent's content: the force path
    // must copy that single child line (a wrong parent view would find
    // nothing there).
    let body = Rect::new(0, 0, 80, 24);
    let hits = empty_hits(body);
    let store = StubStore;

    let mut scroll = 0u32;
    let mut follow = true;
    let mut selection: Option<SelRange> = None;
    let mut subagent_focus = Some(sub_idx);
    let mut subagent_sys = 0u64;
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let mut copy_msg: Option<String> = None;
    let mut last_click: Option<std::time::Instant> = None;
    let mut dbl_click = false;
    let mut queue_scroll: u32 = 0;

    // Screen row 12 = child row 11 (inner_y = 1), beyond parent rows.
    for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        let ev = MouseEvent {
            kind,
            column: 10,
            row: 12,
            modifiers: KeyModifiers::SHIFT,
        };
        handle_mouse(
            ev,
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
    }
    assert!(
        copy_msg.is_some(),
        "shift+click in subagent view must copy the line"
    );
    assert!(
        !copy_msg
            .as_deref()
            .is_some_and(|m| m.contains("Nothing to copy")),
        "shift+click on a child-only row should copy, got: {copy_msg:?}"
    );
    assert!(selection.is_none(), "selection cleared after release");
}
