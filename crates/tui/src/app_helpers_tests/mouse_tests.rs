use super::mouse_helpers::{empty_hits, StubStore};
use crate::app_helpers::*;
use crate::render::SubagentBtn;
use opencoder_session::SessionEvent;
use ratatui::layout::Rect;

#[tokio::test]
async fn dbl_click_selects_line_and_copies_on_release() {
    // Build a chat view with 5 marker lines (abs rows 0-4).
    let mut chat = ChatView::default();
    for &l in &[
        "line one",
        "line two",
        "line three",
        "line four",
        "line five",
    ] {
        chat.push_marker(Line::from(l.to_string()));
    }

    // Body rect: inner_y=1, inner_h=10, so screen row 5 maps to abs row 4.
    let body = Rect::new(0, 0, 80, 12);
    let hits = empty_hits(body);

    let mut scroll = 0u32;
    let mut follow = true;
    let mut selection: Option<SelRange> = None;
    let mut subagent_focus: Option<usize> = None;
    let mut subagent_sys = 0u64;
    let mut queue_items: Vec<(i64, String)> = vec![];
    let mut copy_msg: Option<String> = None;
    let mut last_click: Option<Instant> = None;
    let mut dbl_click = false;
    let mut queue_scroll: u32 = 0;
    let store = StubStore;

    let mk_down = |row| MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 10,
        row,
        modifiers: KeyModifiers::NONE,
    };

    // First click — should NOT set dbl_click.
    handle_mouse(
        mk_down(5),
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
    assert!(!dbl_click, "first click should not be a double-click");

    // Second click immediately — should set dbl_click and selection.
    handle_mouse(
        mk_down(5),
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
    assert!(dbl_click, "second click should be detected as double-click");
    assert!(selection.is_some(), "selection should be set on dbl-click");

    // Mouse up — should copy (force=true via dbl_click).
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
    assert!(copy_msg.is_some(), "double-click should copy on release");
    assert!(selection.is_none(), "selection cleared after release");
    assert!(!dbl_click, "dbl_click reset after release");
}

#[tokio::test]
async fn submit_btn_returns_steer_submit() {
    let mut chat = ChatView::default();
    let body = Rect::new(0, 0, 80, 12);

    // Build a MouseHits with a Submit button for steer seq=10 at (77, 0).
    let mut hits = empty_hits(body);
    hits.queue_btns.push(queue_panel::QueueBtn {
        seq: 10,
        action: queue_panel::QueueBtnAction::Submit,
        rect: Rect::new(77, 0, 1, 1),
    });

    let mut scroll = 0u32;
    let mut follow = true;
    let mut selection: Option<SelRange> = None;
    let mut subagent_focus: Option<usize> = None;
    let mut subagent_sys = 0u64;
    chat.steer_items = vec![(10, "redirect".into())];
    let mut queue_items: Vec<(i64, String)> = vec![];
    let store = StubStore;
    let mut copy_msg: Option<String> = None;
    let mut last_click: Option<Instant> = None;
    let mut dbl_click = false;
    let mut queue_scroll: u32 = 0;

    let down = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 77,
        row: 0,
        modifiers: KeyModifiers::NONE,
    };
    let outcome = handle_mouse(
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

    assert_eq!(
        outcome,
        MouseOutcome::SteerSubmit,
        "clicking Submit on a steer row must return SteerSubmit"
    );
    // Steer item must NOT be removed — promotion happens in the drain loop.
    assert_eq!(
        chat.steer_items.len(),
        1,
        "steer item should remain until drain"
    );
}

#[tokio::test]
async fn single_click_does_not_copy_on_release() {
    let mut chat = ChatView::default();
    for &l in &[
        "line one",
        "line two",
        "line three",
        "line four",
        "line five",
    ] {
        chat.push_marker(Line::from(l.to_string()));
    }

    let body = Rect::new(0, 0, 80, 12);
    let hits = empty_hits(body);

    let mut scroll = 0u32;
    let mut follow = true;
    let mut selection: Option<SelRange> = None;
    let mut subagent_focus: Option<usize> = None;
    let mut subagent_sys = 0u64;
    let mut queue_items: Vec<(i64, String)> = vec![];
    let mut copy_msg: Option<String> = None;
    let mut last_click: Option<Instant> = None;
    let mut dbl_click = false;
    let mut queue_scroll: u32 = 0;
    let store = StubStore;

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
    assert!(!dbl_click);

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
    assert!(copy_msg.is_none(), "single click should not copy");
}

/// Regression: clicking the follow/jump button immediately after a body
/// click must still work. Previously the `jump_btn` check sat AFTER the
/// double-click guard, so the second click (within 400 ms) was swallowed
/// by `is_dbl` and the early `return`, making the follow button
/// unreliable.
#[tokio::test]
async fn jump_btn_click_works_after_recent_body_click() {
    let mut chat = ChatView::default();
    chat.push_marker(Line::from("some text"));

    let body = Rect::new(0, 0, 80, 12);
    // jump_btn sits on the body's bottom-border row, right-aligned.
    let jump_btn_rect = Rect::new(74, 11, 6, 1);
    let hits = MouseHits {
        jump_btn: Some(jump_btn_rect),
        top_btn: None,
        body: Some(body),
        queue_panel: None,
        queue_total: 0,
        queue_btns: Vec::new(),
        thinking_btns: Vec::new(),
        subagent_btns: Vec::new(),
        tool_btns: Vec::new(),
        compaction_btns: Vec::new(),
        total_rows: 0,
    };

    let mut scroll = 0u32;
    let mut follow = false;
    let mut selection: Option<SelRange> = None;
    let mut subagent_focus: Option<usize> = None;
    let mut subagent_sys = 0u64;
    let mut queue_items: Vec<(i64, String)> = vec![];
    let mut copy_msg: Option<String> = None;
    let mut last_click: Option<Instant> = None;
    let mut dbl_click = false;
    let mut queue_scroll: u32 = 0;
    let store = StubStore;

    // First click: hits the body interior (row 5, well inside body).
    let body_click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 10,
        row: 5,
        modifiers: KeyModifiers::NONE,
    };
    handle_mouse(
        body_click,
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
    assert!(last_click.is_some(), "body click should set last_click");
    assert!(!follow, "body click should not set follow");

    // Second click immediately after (< 400 ms): hits the jump button.
    // Under the old code this was swallowed by the double-click guard.
    let jump_click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 76,
        row: 11,
        modifiers: KeyModifiers::NONE,
    };
    handle_mouse(
        jump_click,
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
    assert!(
        follow,
        "jump button click must set follow=true even right after a body click"
    );
}

/// Regression: clicking a Thinking-block header must toggle on the FIRST
/// click even when it lands within the 400 ms double-click window of a
/// previous click. Previously the thinking-toggle loop sat AFTER the
/// dbl-click guard, so any header click within 400 ms of a prior click was
/// swallowed by the guard's early `return` (selecting a line instead) and
/// the toggle never ran — making expansion probabilistic. The fix moves
/// queue/thinking/subagent button-hit detection ahead of the guard, the
/// same fix jump_btn/top_btn already had.

#[tokio::test]
async fn thinking_header_toggles_even_right_after_another_click() {
    let mut chat = ChatView::default();
    chat.apply(&SessionEvent::ReasoningDelta(
        "secret reasoning here".into(),
    ));
    chat.apply(&SessionEvent::TextDelta("answer".into()));
    chat.apply(&SessionEvent::Done);
    // Collapsed by default: the reasoning content must NOT be visible yet.
    assert!(
        !chat.flatten().iter().any(|l| l
            .spans
            .iter()
            .any(|s| s.content.contains("secret reasoning"))),
        "precondition: thinking must start collapsed"
    );

    let body = Rect::new(0, 0, 80, 12);
    let header_rect = Rect::new(1, 1, 78, 1);
    let hits = MouseHits {
        jump_btn: None,
        top_btn: None,
        body: Some(body),
        queue_panel: None,
        queue_total: 0,
        queue_btns: Vec::new(),
        thinking_btns: vec![crate::render::ThinkingBtn {
            block_idx: 0,
            rect: header_rect,
        }],
        subagent_btns: Vec::new(),
        tool_btns: Vec::new(),
        compaction_btns: Vec::new(),
        total_rows: 0,
    };

    let mut scroll = 0u32;
    let mut follow = false;
    let mut selection: Option<SelRange> = None;
    let mut subagent_focus: Option<usize> = None;
    let mut subagent_sys = 0u64;
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let store = StubStore;
    let mut copy_msg: Option<String> = None;
    // A click ~50 ms ago — squarely inside the 400 ms dbl-click window.
    // On the buggy code this trips `is_dbl` and the toggle is skipped.
    let mut last_click: Option<Instant> = Some(Instant::now());
    let mut dbl_click = false;
    let mut queue_scroll: u32 = 0;

    let outcome = handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: header_rect.x,
            row: header_rect.y,
            modifiers: KeyModifiers::NONE,
        },
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
    assert_eq!(outcome, MouseOutcome::None);
    assert!(
        chat.flatten().iter().any(|l| l
            .spans
            .iter()
            .any(|s| s.content.contains("secret reasoning"))),
        "thinking must be expanded after the header click"
    );
    assert!(
        !dbl_click,
        "a header toggle must not be flagged as a double-click"
    );
}

/// Clicking a Compaction-block header must toggle its collapse on the first
/// click. Mirrors `thinking_header_toggles_even_right_after_another_click`.
#[tokio::test]
async fn compaction_header_click_toggles_collapse() {
    use std::time::Instant;

    let mut chat = crate::chat::ChatView::default();
    chat.apply(&SessionEvent::TextDelta("answer".into()));
    chat.apply(&SessionEvent::Done);
    chat.apply(&SessionEvent::Compaction("hidden summary".into()));
    // Collapsed by default.
    assert!(
        !chat
            .flatten()
            .iter()
            .any(|l| l.spans.iter().any(|s| s.content.contains("hidden summary"))),
        "precondition: compaction must start collapsed"
    );

    let body = Rect::new(0, 0, 80, 12);
    let header_rect = Rect::new(1, 2, 78, 1);
    let hits = MouseHits {
        jump_btn: None,
        top_btn: None,
        body: Some(body),
        queue_panel: None,
        queue_total: 0,
        queue_btns: Vec::new(),
        thinking_btns: Vec::new(),
        subagent_btns: Vec::new(),
        tool_btns: Vec::new(),
        compaction_btns: vec![crate::render::CompactionBtn {
            block_idx: crate::chat::ChatView::compaction_headers(&chat)[0].block_idx,
            rect: header_rect,
        }],
        total_rows: 0,
    };

    let mut scroll = 0u32;
    let mut follow = false;
    let mut selection: Option<crate::selection::SelRange> = None;
    let mut subagent_focus: Option<usize> = None;
    let mut subagent_sys = 0u64;
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let store = StubStore;
    let mut copy_msg: Option<String> = None;
    let mut last_click: Option<Instant> = Some(Instant::now());
    let mut dbl_click = false;
    let mut queue_scroll: u32 = 0;

    let outcome = handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: header_rect.x,
            row: header_rect.y,
            modifiers: KeyModifiers::NONE,
        },
        &hits,
        &mut scroll,
        &mut follow,
        &mut selection,
        &mut chat,
        &mut subagent_focus,
        &mut subagent_sys,
        std::path::Path::new("."),
        &mut queue_items,
        "s",
        &store,
        &mut copy_msg,
        &mut last_click,
        &mut dbl_click,
        &mut queue_scroll,
    )
    .await;
    assert_eq!(outcome, MouseOutcome::None);
    assert!(
        chat.flatten()
            .iter()
            .any(|l| l.spans.iter().any(|s| s.content.contains("hidden summary"))),
        "compaction must be expanded after the header click"
    );
}

/// Regression test for the idle-session `[→ view]` bug: when a subagent has
/// finished, clicking its header (which maps to a `SubagentBtn`) must set
/// `subagent_focus` so the child transcript becomes visible. Previously the
/// `Event::Mouse` arm never set `dirty = true`, so the focused view never
/// re-rendered and the click appeared to do nothing.
#[tokio::test]
async fn clicking_subagent_view_enters_subagent() {
    // Build a ChatView with one completed Subagent block — the realistic
    // idle-session scenario where the user clicks `[→ view]`.
    let mut chat = ChatView::default();
    chat.apply(&SessionEvent::SubagentStart {
        id: "s1".into(),
        kind: "explore".into(),
        prompt: "test subagent".into(),
        child_session_id: "c1".into(),
    });
    chat.apply(&SessionEvent::SubagentEnd {
        id: "s1".into(),
        ok: true,
        cancelled: false,
        summary: "done".into(),
    });

    // Body-only hit map with a single subagent header button at row 1.
    let body = Rect::new(0, 0, 80, 12);
    let mut hits = empty_hits(body);
    hits.subagent_btns.push(SubagentBtn {
        block_idx: 0,
        rect: Rect::new(1, 1, 78, 1),
    });

    let mut scroll = 0u32;
    let mut follow = true;
    let mut selection: Option<SelRange> = None;
    let mut subagent_focus: Option<usize> = None;
    let mut subagent_sys = 0u64;
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let mut copy_msg: Option<String> = None;
    let mut last_click: Option<Instant> = None;
    let mut dbl_click = false;
    let mut queue_scroll: u32 = 0;
    let store = StubStore;

    handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row: 1,
            modifiers: KeyModifiers::NONE,
        },
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
        subagent_focus,
        Some(0),
        "clicking a subagent header must enter the subagent view"
    );
}
