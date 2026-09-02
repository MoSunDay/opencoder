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
        tool_btns: Vec::new(),
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
        attach_del_btns: Vec::new(),
        thinking_btns: vec![crate::render::ThinkingBtn {
            block_idx: 0,
            rect: header_rect,
        }],
        subagent_btns: Vec::new(),
        tool_btns: Vec::new(),
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
        tool_btns: Vec::new(),
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
    let mut subagent_focus: Option<usize> = None;
    let mut subagent_sys = 0u64;
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let mut queue_scroll: u32 = 0;
    let store = StubStore::default();

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
        subagent_focus,
        Some(0),
        "clicking a subagent header must enter the subagent view"
    );
}

/// Clicking an attachment ✕ button removes exactly the clicked pending image
/// (the first-click region, like queue buttons — no recent-body guard).
#[tokio::test]
async fn attach_del_click_removes_only_clicked_image() {
    let mut chat = ChatView::default();
    let body = Rect::new(0, 0, 80, 12);
    let mut hits = empty_hits(body);
    hits.attach_del_btns
        .push(crate::attach_badge::AttachDelBtn {
            index: 0,
            rect: Rect::new(78, 0, 1, 1),
        });
    hits.attach_del_btns
        .push(crate::attach_badge::AttachDelBtn {
            index: 1,
            rect: Rect::new(78, 1, 1, 1),
        });
    let mut pending_images: Vec<(String, String)> = vec![
        ("data:image/png;base64,aa".to_string(), "a.png".to_string()),
        ("data:image/png;base64,bb".to_string(), "b.png".to_string()),
    ];

    let mut scroll = 0u32;
    let mut follow = true;
    let mut subagent_focus: Option<usize> = None;
    let mut subagent_sys = 0u64;
    let mut queue_items: Vec<(i64, String)> = vec![];
    let store = StubStore::default();
    let mut queue_scroll: u32 = 0;

    // Click the SECOND ✕ (row 1).
    let down = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 78,
        row: 1,
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
        &mut pending_images,
    )
    .await;

    assert_eq!(outcome, MouseOutcome::None);
    assert_eq!(pending_images.len(), 1, "exactly one attachment removed");
    assert_eq!(
        pending_images[0].1, "a.png",
        "the FIRST image must survive a click on the second ✕"
    );
    assert_eq!(chat.steer_items.len(), 0, "no steer side effects");
}

/// Clicking a ToolGroup's group line cycles the three display states
/// Collapsed → List → Results → Collapsed. This exercises the exact
/// `tool_btns` → `handle_mouse` → `cycle_tool_group_at` wiring the renderer
/// feeds.
#[tokio::test]
async fn clicking_tool_group_line_cycles_three_states() {
    use crate::chat::{ChatBlock, ToolGroupState};
    use crate::render::ToolBtn;

    let mut chat = ChatView::default();
    chat.apply(&SessionEvent::ToolStart {
        id: "t1".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "ls"}),
    });
    chat.apply(&SessionEvent::ToolEnd {
        id: "t1".into(),
        name: "bash".into(),
        output: "done".into(),
        is_error: false,
        images: Vec::new(),
    });

    let state = |c: &ChatView| -> ToolGroupState {
        match c.blocks.first() {
            Some(ChatBlock::ToolGroup { state, .. }) => *state,
            other => panic!("expected a ToolGroup first, got {other:?}"),
        }
    };
    assert!(matches!(state(&chat), ToolGroupState::Collapsed));

    async fn drive(chat: &mut ChatView, row: u16) {
        let body = Rect::new(0, 0, 80, 12);
        let mut hits = empty_hits(body);
        hits.tool_btns.push(ToolBtn {
            block_idx: 0,
            rect: Rect::new(0, 5, 80, 1),
        });
        let click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 3,
            row,
            modifiers: KeyModifiers::NONE,
        };
        let mut scroll = 0u32;
        let mut follow = true;
        let mut subagent_focus: Option<usize> = None;
        let mut subagent_sys = 0u64;
        let mut queue_items: Vec<(i64, String)> = vec![];
        let store = StubStore::default();
        let mut queue_scroll: u32 = 0;
        let mut pending_images = vec![];
        handle_mouse(
            click,
            &hits,
            &mut scroll,
            &mut follow,
            chat,
            &mut subagent_focus,
            &mut subagent_sys,
            Path::new("."),
            &mut queue_items,
            "s",
            &store,
            &mut queue_scroll,
            &mut pending_images,
        )
        .await;
    }

    drive(&mut chat, 5).await; // first click: Collapsed -> List
    assert!(matches!(state(&chat), ToolGroupState::List));
    drive(&mut chat, 5).await; // second click: List -> Results
    assert!(matches!(state(&chat), ToolGroupState::Results));
    drive(&mut chat, 5).await; // third click: Results -> Collapsed
    assert!(matches!(state(&chat), ToolGroupState::Collapsed));

    // A click outside every button rect must not cycle anything.
    drive(&mut chat, 0).await;
    assert!(
        matches!(state(&chat), ToolGroupState::Collapsed),
        "miss-click must not cycle the group"
    );
}

/// Clicking a tool-call header row (List state) toggles ONLY that call's
/// output. Exercises the exact `tool_call_btns` → `handle_mouse` →
/// `toggle_tool_call_at` wiring the renderer feeds, and proves the call-row
/// dispatch precedes the group-line cycle.
#[tokio::test]
async fn clicking_tool_call_row_toggles_only_that_call() {
    use crate::chat::{ChatBlock, ToolGroupState};
    use crate::render::{ToolBtn, ToolCallBtn};

    let mut chat = ChatView::default();
    for (id, cmd) in [("a", "echo A"), ("b", "echo B")] {
        chat.apply(&SessionEvent::ToolStart {
            id: id.into(),
            name: "bash".into(),
            input: serde_json::json!({"command": cmd}),
        });
        chat.apply(&SessionEvent::ToolEnd {
            id: id.into(),
            name: "bash".into(),
            output: format!("{id}-out"),
            is_error: false,
            images: Vec::new(),
        });
    }
    chat.cycle_tool_group_at(0); // Collapsed -> List
    let state = |c: &ChatView| match c.blocks.first() {
        Some(ChatBlock::ToolGroup { state, .. }) => *state,
        other => panic!("expected a ToolGroup first, got {other:?}"),
    };
    let expanded = |c: &ChatView| match c.blocks.first() {
        Some(ChatBlock::ToolGroup { calls, .. }) => {
            calls.iter().map(|x| x.expanded).collect::<Vec<_>>()
        }
        other => panic!("expected a ToolGroup first, got {other:?}"),
    };
    assert!(matches!(state(&chat), ToolGroupState::List));

    // Call header rows sit on rows 1 (call 0) and 2 (call 1) of the body.
    // `kind` picks which button list gets the rect; `click_row` is where the
    // synthetic click lands.
    async fn drive(
        chat: &mut ChatView,
        kind: &str,
        call_idx: usize,
        rect_row: u16,
        click_row: u16,
    ) {
        let body = Rect::new(0, 0, 80, 12);
        let mut hits = empty_hits(body);
        match kind {
            "call" => hits.tool_call_btns.push(ToolCallBtn {
                block_idx: 0,
                call_idx,
                rect: Rect::new(0, rect_row, 80, 1),
            }),
            _ => hits.tool_btns.push(ToolBtn {
                block_idx: 0,
                rect: Rect::new(0, rect_row, 80, 1),
            }),
        }
        let click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 3,
            row: click_row,
            modifiers: KeyModifiers::NONE,
        };
        let mut scroll = 0u32;
        let mut follow = true;
        let mut subagent_focus: Option<usize> = None;
        let mut subagent_sys = 0u64;
        let mut queue_items: Vec<(i64, String)> = vec![];
        let store = StubStore::default();
        let mut queue_scroll: u32 = 0;
        let mut pending_images = vec![];
        handle_mouse(
            click,
            &hits,
            &mut scroll,
            &mut follow,
            chat,
            &mut subagent_focus,
            &mut subagent_sys,
            Path::new("."),
            &mut queue_items,
            "s",
            &store,
            &mut queue_scroll,
            &mut pending_images,
        )
        .await;
    }

    // Click call 0's header row: toggles call 0 only, state stays List.
    drive(&mut chat, "call", 0, 1, 1).await;
    assert_eq!(expanded(&chat), vec![true, false]);
    assert!(matches!(state(&chat), ToolGroupState::List));

    // Click call 1's header row: toggles call 1 only.
    drive(&mut chat, "call", 1, 2, 2).await;
    assert_eq!(expanded(&chat), vec![true, true]);

    // Click call 0's row again: collapses just call 0.
    drive(&mut chat, "call", 0, 1, 1).await;
    assert_eq!(expanded(&chat), vec![false, true]);

    // A group-line click still cycles the group (List -> Results) and leaves
    // the per-call flags untouched.
    drive(&mut chat, "group", 0, 0, 0).await;
    assert!(matches!(state(&chat), ToolGroupState::Results));
    assert_eq!(expanded(&chat), vec![false, true]);
}

/// Regression: control-strip hit rects are 2 cells wide (separator space +
/// glyph), so a click one column left of the visible glyph still lands.
/// Under the old 1×1 rects, any pad drift left both buttons pointing at
/// blank cells — and dead.
#[tokio::test]
async fn steer_delete_click_on_separator_space_still_hits() {
    let mut chat = ChatView::default();
    let body = Rect::new(0, 0, 80, 12);
    let mut hits = empty_hits(body);
    // Same geometry the renderer emits: delete glyph at width-3, rect
    // spanning [width-4, width-3].
    let del_x = queue_panel::steer_btn_x_offsets(80)[0];
    let rect = queue_panel::glyph_hit_rect(0, del_x, 0);
    assert_eq!((rect.x, rect.width), (76, 2));
    hits.queue_btns.push(queue_panel::QueueBtn {
        seq: 10,
        action: queue_panel::QueueBtnAction::Delete,
        rect,
    });

    chat.steer_items = vec![(10, "redirect".into())];
    let store = StubStore::default();
    let mut scroll = 0u32;
    let mut follow = true;
    let mut subagent_focus: Option<usize> = None;
    let mut subagent_sys = 0u64;
    let mut queue_items: Vec<(i64, String)> = vec![];
    let mut queue_scroll: u32 = 0;

    // Click the separator-space cell, one column left of the ✕ glyph.
    let click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: rect.x,
        row: 0,
        modifiers: KeyModifiers::NONE,
    };
    let outcome = handle_mouse(
        click,
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
        &mut vec![],
    )
    .await;

    assert_eq!(outcome, MouseOutcome::None);
    assert!(
        chat.steer_items.is_empty(),
        "clicking the separator space must still delete the row"
    );
    assert_eq!(*store.deleted.lock().unwrap(), vec![10]);
}

/// Regression: while a live subagent is focused, the panel renders the CHILD
/// view's steer mirror; a successful delete must retain that mirror too, or
/// the clicked row lingers on screen as a ghost.
#[tokio::test]
async fn steer_delete_removes_focused_subagent_mirror_row() {
    let mut chat = ChatView::default();
    chat.apply(&SessionEvent::SubagentStart {
        id: "s1".into(),
        kind: "explore".into(),
        prompt: "go".into(),
        child_session_id: "c1".into(),
    });
    let focus = chat.blocks.len() - 1;
    match chat.blocks.get_mut(focus) {
        Some(crate::chat::ChatBlock::Subagent { view, .. }) => {
            view.steer_items.push((33, "child steer".into()));
        }
        _ => panic!("expected a subagent block"),
    }
    // A parent row that must survive: the delete targets the child's seq.
    chat.steer_items = vec![(44, "parent steer".into())];

    let mut hits = empty_hits(Rect::new(0, 0, 80, 12));
    hits.queue_btns.push(queue_panel::QueueBtn {
        seq: 33,
        action: queue_panel::QueueBtnAction::Delete,
        rect: Rect::new(76, 0, 2, 1),
    });

    let store = StubStore::default();
    let mut scroll = 0u32;
    let mut follow = true;
    let mut subagent_focus: Option<usize> = Some(focus);
    let mut subagent_sys = 0u64;
    let mut queue_items: Vec<(i64, String)> = vec![];
    let mut queue_scroll: u32 = 0;

    let click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 77,
        row: 0,
        modifiers: KeyModifiers::NONE,
    };
    handle_mouse(
        click,
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
        &mut vec![],
    )
    .await;

    assert_eq!(*store.deleted.lock().unwrap(), vec![33]);
    match chat.blocks.get(focus) {
        Some(crate::chat::ChatBlock::Subagent { view, .. }) => assert!(
            view.steer_items.is_empty(),
            "child mirror row must be removed"
        ),
        _ => panic!("expected a subagent block"),
    }
    assert_eq!(
        chat.steer_items,
        vec![(44, "parent steer".to_string())],
        "parent mirror must be untouched by a child-row delete"
    );
}
