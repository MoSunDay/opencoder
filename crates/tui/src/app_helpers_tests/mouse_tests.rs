use super::mouse_helpers::{empty_hits, StubStore};
use crate::app_helpers::*;
use crate::queue_panel;
use crate::render::{MouseHits, SubagentBtn};
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use opencoder_session::SessionEvent;
use ratatui::layout::Rect;

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
    let mut subagent_focus: Option<usize> = None;
    let mut subagent_sys = 0u64;
    chat.steer_items = vec![(10, "redirect".into())];
    let mut queue_items: Vec<(i64, String)> = vec![];
    let store = StubStore::default();
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
        attach_del_btns: Vec::new(),
        thinking_btns: Vec::new(),
        subagent_btns: Vec::new(),
        tool_call_btns: Vec::new(),
        compaction_btns: Vec::new(),
        keymap_btns: Vec::new(),
        total_rows: 0,
    };

    let mut scroll = 0u32;
    let mut follow = false;
    let mut subagent_focus: Option<usize> = None;
    let mut subagent_sys = 0u64;
    let mut queue_items: Vec<(i64, String)> = vec![];
    let mut queue_scroll: u32 = 0;
    let store = StubStore::default();

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
    // Legacy/replay shape built directly (live reasoning goes into the
    // ladder): the double-click-window fix is header-machinery-generic.
    chat.blocks.push(crate::chat::ChatBlock::Thinking {
        text: "secret reasoning here".into(),
        collapsed: true,
        sealed: true,
    });
    chat.apply(&SessionEvent::TextDelta("answer".into()));
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
        attach_del_btns: Vec::new(),
        thinking_btns: vec![crate::render::ThinkingBtn {
            block_idx: 0,
            rect: header_rect,
        }],
        subagent_btns: Vec::new(),
        tool_call_btns: Vec::new(),
        compaction_btns: Vec::new(),
        keymap_btns: Vec::new(),
        total_rows: 0,
    };

    let mut scroll = 0u32;
    let mut follow = false;
    let mut subagent_focus: Option<usize> = None;
    let mut subagent_sys = 0u64;
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let store = StubStore::default();
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
    assert_eq!(outcome, MouseOutcome::None);
    assert!(
        chat.flatten().iter().any(|l| l
            .spans
            .iter()
            .any(|s| s.content.contains("secret reasoning"))),
        "thinking must be expanded after the header click"
    );
}

/// Clicking a Compaction-block header must toggle its collapse on the first
/// click. Mirrors `thinking_header_toggles_even_right_after_another_click`.
#[tokio::test]
async fn compaction_header_click_toggles_collapse() {
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
        attach_del_btns: Vec::new(),
        thinking_btns: Vec::new(),
        subagent_btns: Vec::new(),
        tool_call_btns: Vec::new(),
        compaction_btns: vec![crate::render::CompactionBtn {
            block_idx: crate::chat::ChatView::compaction_headers(&chat)[0].block_idx,
            rect: header_rect,
        }],
        keymap_btns: Vec::new(),
        total_rows: 0,
    };

    let mut scroll = 0u32;
    let mut follow = false;
    let mut subagent_focus: Option<usize> = None;
    let mut subagent_sys = 0u64;
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let store = StubStore::default();
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
        &mut chat,
        &mut subagent_focus,
        &mut subagent_sys,
        std::path::Path::new("."),
        &mut queue_items,
        "s",
        &store,
        &mut queue_scroll,
        &mut vec![], // no pending images
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

#[path = "mouse_tests/hierarchy_and_actions.rs"]
mod hierarchy_and_actions;
#[path = "mouse_tests/steer_actions.rs"]
mod steer_actions;
