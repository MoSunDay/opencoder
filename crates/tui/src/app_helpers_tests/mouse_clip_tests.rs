//! Tests for clipboard-copy mouse interactions: Shift+drag bypass,
//! multi-line drag selection, double-click on blank lines, and the
//! pure `is_within_dbl_click_window` timing boundary.

use crate::app_helpers::*;
use async_trait::async_trait;
use opencoder_core::Message;
use opencoder_store::{
    Delivery, SessionEventRecord, SessionFilter, SessionInput, SessionListItem, SessionMeta,
    SessionPatch, SubagentTaskRecord,
};
use ratatui::layout::Rect;

/// Minimal `Store` stub whose every method panics. The mouse-copy code paths
/// tested here never touch the store.
struct StubStore;

#[async_trait]
impl opencoder_store::Store for StubStore {
    fn backend_name(&self) -> &'static str {
        "stub"
    }
    async fn create_session(&self, _: &SessionMeta) -> anyhow::Result<()> {
        unimplemented!()
    }
    async fn get_session(&self, _: &str) -> anyhow::Result<Option<SessionMeta>> {
        unimplemented!()
    }
    async fn list_sessions(&self, _: &SessionFilter) -> anyhow::Result<Vec<SessionListItem>> {
        unimplemented!()
    }
    async fn update_session(&self, _: &str, _: &SessionPatch) -> anyhow::Result<()> {
        unimplemented!()
    }
    async fn delete_session(&self, _: &str) -> anyhow::Result<()> {
        unimplemented!()
    }
    async fn clear_other_sessions(&self, _: &str) -> anyhow::Result<u64> {
        unimplemented!()
    }
    async fn append_message(&self, _: &str, _: &Message) -> anyhow::Result<i64> {
        unimplemented!()
    }
    async fn append_messages(&self, _: &str, _: &[Message]) -> anyhow::Result<Vec<i64>> {
        unimplemented!()
    }
    async fn load_messages(&self, _: &str) -> anyhow::Result<Vec<Message>> {
        unimplemented!()
    }
    async fn last_message_seq(&self, _: &str) -> anyhow::Result<i64> {
        unimplemented!()
    }
    async fn admit_input(&self, _: &SessionInput) -> anyhow::Result<i64> {
        unimplemented!()
    }
    async fn pending_inputs(&self, _: &str, _: Delivery) -> anyhow::Result<Vec<SessionInput>> {
        unimplemented!()
    }
    async fn promote_inputs(
        &self,
        _: &str,
        _: i64,
        _: Delivery,
    ) -> anyhow::Result<Vec<i64>> {
        unimplemented!()
    }
    async fn promote_next_queued(&self, _: &str) -> anyhow::Result<Option<i64>> {
        unimplemented!()
    }
    async fn claim_next_queue(
        &self,
        _: &str,
    ) -> anyhow::Result<Option<(i64, SessionInput)>> {
        unimplemented!()
    }
    async fn delete_input(&self, _: i64) -> anyhow::Result<()> {
        unimplemented!()
    }
    async fn swap_input_order(&self, _: &str, _: i64, _: i64) -> anyhow::Result<()> {
        unimplemented!()
    }
    async fn append_events(&self, _: &[SessionEventRecord]) -> anyhow::Result<Vec<i64>> {
        unimplemented!()
    }
    async fn events_after(&self, _: &str, _: i64) -> anyhow::Result<Vec<SessionEventRecord>> {
        unimplemented!()
    }
    async fn last_event_seq(&self, _: &str) -> anyhow::Result<i64> {
        unimplemented!()
    }
    async fn create_subagent_task(&self, _: &SubagentTaskRecord) -> anyhow::Result<()> {
        unimplemented!()
    }
    async fn complete_subagent_task(&self, _: &str, _: &str, _: bool) -> anyhow::Result<()> {
        unimplemented!()
    }
    async fn list_subagent_tasks(
        &self,
        _: &str,
    ) -> anyhow::Result<Vec<SubagentTaskRecord>> {
        unimplemented!()
    }
    async fn get_subagent_task(&self, _: &str) -> anyhow::Result<Option<SubagentTaskRecord>> {
        unimplemented!()
    }
    async fn cancel_subagent_task(&self, _: &str) -> anyhow::Result<()> {
        unimplemented!()
    }
}

fn empty_hits(body: Rect) -> MouseHits {
    MouseHits {
        jump_btn: None,
        top_btn: None,
        body: Some(body),
        queue_btns: Vec::new(),
        thinking_btns: Vec::new(),
        subagent_btns: Vec::new(),
        tool_btns: Vec::new(),
        total_rows: 0,
    }
}

/// Build a ChatView whose flattened lines are exactly the given strings
/// (one Marker block per line), so tests are independent of the markdown
/// renderer.
fn view_from_lines(lines: &[&str]) -> ChatView {
    let mut v = ChatView::default();
    for &l in lines {
        v.push_marker(ratatui::text::Line::from(l.to_string()));
    }
    v
}

// ── Shift+drag bypass ──────────────────────────────────────────────

#[tokio::test]
async fn shift_drag_down_clears_selection_and_returns_none() {
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

    // First establish a selection with a normal click at screen row 5.
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
    )
    .await;
    assert!(selection.is_some(), "normal click should start selection");

    // Now Shift+Down must clear it and return None.
    let shift_down = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 10,
        row: 5,
        modifiers: KeyModifiers::SHIFT,
    };
    let outcome = handle_mouse(
        shift_down, &hits, &mut scroll, &mut follow, &mut selection, &mut chat,
        &mut subagent_focus, &mut parent_scroll, &mut parent_follow,
        &mut subagent_sys, Path::new("."), &mut queue_items, "s", &store,
        &mut copy_msg, &mut last_click, &mut dbl_click,
    )
    .await;
    assert_eq!(outcome, MouseOutcome::None);
    assert!(selection.is_none(), "Shift+Down must clear selection");
}

#[tokio::test]
async fn shift_drag_does_not_copy_on_release() {
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

    // Shift+Down (bypassed), Shift+Drag (bypassed), then Up.
    for (kind, row) in [
        (MouseEventKind::Down(MouseButton::Left), 5u16),
        (MouseEventKind::Drag(MouseButton::Left), 7u16),
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
        )
        .await;
    }
    assert!(selection.is_none(), "Shift+drag must not create selection");

    // Mouse up - should NOT copy because selection is None.
    let up = MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        column: 10,
        row: 7,
        modifiers: KeyModifiers::SHIFT,
    };
    handle_mouse(
        up, &hits, &mut scroll, &mut follow, &mut selection, &mut chat,
        &mut subagent_focus, &mut parent_scroll, &mut parent_follow,
        &mut subagent_sys, Path::new("."), &mut queue_items, "s", &store,
        &mut copy_msg, &mut last_click, &mut dbl_click,
    )
    .await;
    assert!(copy_msg.is_none(), "Shift+drag release must not trigger copy");
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
    )
    .await;

    assert!(copy_msg.is_some(), "drag should copy on release");
    assert!(selection.is_none(), "selection cleared after release");
}

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
