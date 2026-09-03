use super::*;

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
