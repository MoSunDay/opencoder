use super::*;

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

/// Clicking a rendered ladder row toggles ONLY that target. Exercises the
/// exact `tool_call_btns` → `handle_mouse` → `toggle_tool_call_at` wiring
/// the renderer feeds. `call_idx` indexes the group's VISIBLE rows: the
/// group row, then (while open) each step row, then (while the step is
/// open) its calls aggregation row, then each function-call row while that
/// list is open — in render order.
#[tokio::test]
async fn clicking_step_or_call_row_toggles_only_that_target() {
    use crate::chat::ChatBlock;
    use crate::render::ToolCallBtn;

    let mut chat = ChatView::default();
    for (id, cmd) in [("a", "echo A"), ("b", "echo B")] {
        chat.apply(&SessionEvent::ReasoningDelta(format!("think {id}")));
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
    let state = |c: &ChatView| match c.blocks.first() {
        Some(ChatBlock::StepGroup { steps, open, .. }) => (
            *open,
            steps.iter().map(|s| s.open).collect::<Vec<_>>(),
            steps.iter().map(|s| s.calls_open).collect::<Vec<_>>(),
            steps
                .iter()
                .flat_map(|s| s.calls.iter().map(|x| x.expanded))
                .collect::<Vec<_>>(),
        ),
        other => panic!("expected a StepGroup first, got {other:?}"),
    };
    assert_eq!(
        state(&chat),
        (
            false,
            vec![false, false],
            vec![false, false],
            vec![false, false]
        ),
        "whole ladder starts collapsed"
    );

    // `call_idx` picks the target row; `rect_row` is the rect's screen row
    // and `click_row` is where the synthetic click lands.
    async fn drive(chat: &mut ChatView, call_idx: usize, rect_row: u16, click_row: u16) {
        let body = Rect::new(0, 0, 80, 12);
        let mut hits = empty_hits(body);
        hits.tool_call_btns.push(ToolCallBtn {
            block_idx: 0,
            call_idx,
            rect: Rect::new(0, rect_row, 80, 1),
        });
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

    // L0: collapsed group shows ONLY the group row; clicking it opens the
    // group and reveals the step rows.
    drive(&mut chat, 0, 0, 0).await; // group row -> group opens
    assert_eq!(
        state(&chat),
        (
            true,
            vec![false, false],
            vec![false, false],
            vec![false, false]
        )
    );

    // L1: [Group, Step1, Step2]; click Step(1) -> its ladder level opens.
    drive(&mut chat, 1, 1, 1).await; // Step(1) row -> step opens
    assert_eq!(
        state(&chat),
        (
            true,
            vec![true, false],
            vec![false, false],
            vec![false, false]
        )
    );

    // L2: [Turn, Step1, Calls1, Step2]; open the first call list.
    drive(&mut chat, 2, 2, 2).await;
    assert_eq!(
        state(&chat),
        (
            true,
            vec![true, false],
            vec![true, false],
            vec![false, false]
        )
    );

    // L3: [Turn, Step1, Calls1, call-a, Step2]; open call a's result.
    drive(&mut chat, 3, 3, 3).await;
    assert_eq!(
        state(&chat),
        (
            true,
            vec![true, false],
            vec![true, false],
            vec![true, false]
        )
    );

    // Step(2) follows call a in the visible-target walk.
    drive(&mut chat, 4, 4, 4).await;
    assert_eq!(
        state(&chat),
        (true, vec![true, true], vec![true, false], vec![true, false])
    );

    // Open Step(2)'s call list, then its call result.
    drive(&mut chat, 5, 5, 5).await;
    drive(&mut chat, 6, 6, 6).await;
    assert_eq!(
        state(&chat),
        (true, vec![true, true], vec![true, true], vec![true, true])
    );

    // Click call b again: collapses just its result.
    drive(&mut chat, 6, 6, 6).await;
    assert_eq!(
        state(&chat),
        (true, vec![true, true], vec![true, true], vec![true, false])
    );

    // Click the group row again: the group folds away (state below is kept
    // but hidden — one click re-reveals the whole ladder).
    drive(&mut chat, 0, 0, 0).await;
    assert_eq!(
        state(&chat),
        (false, vec![true, true], vec![true, true], vec![true, false]),
        "group toggle must not reset inner ladder state"
    );
    let flat = chat.flatten();
    let row: String = flat[0].spans.iter().map(|s| s.content.clone()).collect();
    assert!(
        row.starts_with("\u{25b8} "),
        "group row re-collapsed: {row:?}"
    );
    assert_eq!(flat.len(), 2, "collapsed group = group row + blank");
}

/// While the sidecar panel is focused, its body renders the panel's nested
/// view — step-row hit rects carry PANEL-relative block indices. The click
/// must toggle the panel view's ladder, never the main transcript behind it
/// (regression: `collapse_view` ignored `sidecar_focus` and toggled the main
/// view at the same index, so the sidecar ladder was click-dead and a same-
/// index main group silently flipped).
#[tokio::test]
async fn sidecar_step_row_click_toggles_the_panel_view_not_the_main_transcript() {
    let mut chat = ChatView::default();
    // Main transcript carries its own (different) collapsed group at the
    // same block index — it must stay untouched by the panel click.
    chat.apply(&SessionEvent::ReasoningDelta("main think".into()));
    chat.apply(&SessionEvent::ToolStart {
        id: "m1".into(),
        name: "bash".into(),
        input: "echo main".into(),
    });

    let (tx, _rx) = tokio::sync::mpsc::channel::<crate::sidecar_ui::SidecarCmd>(1);
    crate::sidecar_ui::enter_panel(&mut chat, &tx);
    crate::sidecar_ui::echo_question(&mut chat, "面板问题");
    chat.apply(&SessionEvent::SidecarStart {
        id: "sc-1".into(),
        question: "面板问题".into(),
    });
    chat.apply(&SessionEvent::SidecarChild {
        id: "sc-1".into(),
        ev: Box::new(SessionEvent::ReasoningDelta("panel think".into())),
    });
    chat.apply(&SessionEvent::SidecarChild {
        id: "sc-1".into(),
        ev: Box::new(SessionEvent::ToolStart {
            id: "p1".into(),
            name: "bash".into(),
            input: "echo panel".into(),
        }),
    });

    // Body hit map recorded against the RENDERED (panel) view: the group row
    // carries the panel-relative block index of the ladder (behind the echoed
    // prompt) and visible-target 0 (the group row).
    let panel_block_idx = chat
        .sidecar
        .as_ref()
        .and_then(|p| {
            p.view
                .blocks
                .iter()
                .position(|b| matches!(b, crate::chat::ChatBlock::StepGroup { .. }))
        })
        .expect("panel ladder exists");
    let body = Rect::new(0, 0, 80, 12);
    let mut hits = empty_hits(body);
    hits.tool_call_btns.push(crate::render::ToolCallBtn {
        block_idx: panel_block_idx,
        call_idx: 0,
        rect: Rect::new(2, 1, 30, 1),
    });

    let mut scroll = 0u32;
    let mut follow = true;
    let mut subagent_focus: Option<usize> = None;
    let mut subagent_sys = 0u64;
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let mut queue_scroll = 0u32;
    let store = StubStore::default();

    handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5,
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
        &mut vec![],
    )
    .await;

    let panel_group_open = chat
        .sidecar
        .as_ref()
        .and_then(|p| {
            p.view.blocks.get(panel_block_idx).and_then(|b| match b {
                crate::chat::ChatBlock::StepGroup { open, .. } => Some(*open),
                _ => None,
            })
        })
        .expect("panel ladder block");
    assert!(
        panel_group_open,
        "the click must open the PANEL view's ladder"
    );
    let main_group_open = matches!(
        chat.blocks.first(),
        Some(crate::chat::ChatBlock::StepGroup { open: true, .. })
    );
    assert!(
        !main_group_open,
        "the main transcript's same-index group must stay collapsed"
    );
}
