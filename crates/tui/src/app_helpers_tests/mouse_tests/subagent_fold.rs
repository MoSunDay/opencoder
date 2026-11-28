use super::*;

/// Fixture: one finished subagent block at index 0 (one contiguous run) plus
/// a body-only hit map built from the view's OWN header indices, exactly the
/// geometry the recorders emit: the group header on its flattened row, the
/// subagent row right below it (expanded only), `rect.x == body.x`, full
/// text width, one row tall.
fn chat_and_hits(collapsed: bool) -> (ChatView, MouseHits, Rect, Option<Rect>) {
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
    chat.subagents_collapsed = collapsed;
    let body = Rect::new(0, 0, 80, 12);
    let mut hits = empty_hits(body);
    let groups = chat.subagent_group_headers();
    assert_eq!(groups.len(), 1, "one group header per run, in both states");
    let group_rect = Rect::new(1, 1 + groups[0].header_line_idx as u16, 78, 1);
    hits.subagent_group_btns
        .push(SubagentGroupBtn { rect: group_rect });
    let rows = chat.subagent_headers();
    let row_rect = rows.first().map(|h| {
        let rect = Rect::new(1, 1 + h.header_line_idx as u16, 78, 1);
        hits.subagent_btns.push(SubagentBtn {
            block_idx: h.block_idx,
            rect,
        });
        rect
    });
    (chat, hits, group_rect, row_rect)
}

/// The view's flattened rows as plain strings (span contents joined).
fn flat_lines(v: &ChatView) -> Vec<String> {
    v.flatten()
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.clone()).collect())
        .collect()
}

#[allow(clippy::too_many_arguments)]
async fn click_at(
    hits: &MouseHits,
    chat: &mut ChatView,
    subagent_focus: &mut Option<usize>,
    scroll: &mut u32,
    follow: &mut bool,
    column: u16,
    row: u16,
) -> MouseOutcome {
    let mut subagent_sys = 0u64;
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let mut queue_scroll = 0u32;
    let store = StubStore::default();
    handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        },
        hits,
        scroll,
        follow,
        chat,
        subagent_focus,
        &mut subagent_sys,
        Path::new("."),
        &mut queue_items,
        "s",
        &store,
        &mut queue_scroll,
        &mut vec![], // no pending images
    )
    .await
}

/// Expanded: the hit map carries the `> subagents` group header AND the
/// subagent row below it; collapsed: only the `▸ subagents` header.
#[test]
fn hit_map_carries_the_group_header_and_rows_only_while_expanded() {
    let (_, hits, _, row) = chat_and_hits(false);
    assert_eq!(hits.subagent_group_btns.len(), 1);
    assert!(
        row.is_some(),
        "the subagent row is a click target while expanded"
    );
    assert_eq!(hits.subagent_btns.len(), 1);
    let group_row = hits.subagent_group_btns[0].rect.y;
    assert_eq!(
        hits.subagent_btns[0].rect.y,
        group_row + 1,
        "row sits below the header"
    );

    let (_, hits, _, row) = chat_and_hits(true);
    assert_eq!(
        hits.subagent_group_btns.len(),
        1,
        "header target stays while collapsed"
    );
    assert!(row.is_none(), "no row target while collapsed");
    assert!(hits.subagent_btns.is_empty());
}

/// Clicking the `> subagents` group header folds EVERY subagent run of the
/// focused view at once — without entering the subagent and without touching
/// scroll or follow. A second click (now on `▸ subagents`) re-expands.
#[tokio::test]
async fn clicking_the_group_header_toggles_the_subagent_rows_without_entering_the_view() {
    let (mut chat, hits, group_rect, _) = chat_and_hits(false);
    assert!(!chat.subagents_collapsed, "expanded by default");
    assert_eq!(
        flat_lines(&chat),
        vec![
            "> subagents",
            "\u{2937} subagent [explore] test subagent \u{2714} done, 0 Steps [\u{2192} view]  done",
        ],
        "expanded: group header, then the full row"
    );

    let mut subagent_focus: Option<usize> = None;
    let mut scroll = 7u32; // deliberately NOT at the top
    let mut follow = false; // deliberately NOT following
    let outcome = click_at(
        &hits,
        &mut chat,
        &mut subagent_focus,
        &mut scroll,
        &mut follow,
        group_rect.x,
        group_rect.y,
    )
    .await;
    assert_eq!(outcome, MouseOutcome::None);
    assert!(
        chat.subagents_collapsed,
        "the group header click must collapse the subagent rows"
    );
    assert_eq!(
        flat_lines(&chat),
        vec!["\u{25b8} subagents"],
        "collapsed: the header is the run's only row"
    );
    assert_eq!(
        subagent_focus, None,
        "the group header click must NOT enter the subagent view"
    );
    assert_eq!(scroll, 7, "the group header click must not touch scroll");
    assert!(!follow, "the group header click must not touch follow");

    // Second click on the (now `▸ subagents`) header re-expands.
    let outcome = click_at(
        &hits,
        &mut chat,
        &mut subagent_focus,
        &mut scroll,
        &mut follow,
        group_rect.x + 1,
        group_rect.y,
    )
    .await;
    assert_eq!(outcome, MouseOutcome::None);
    assert!(
        !chat.subagents_collapsed,
        "the group header click must re-expand the subagent rows"
    );
    assert!(flat_lines(&chat)[0].starts_with("> subagents"));
    assert_eq!(subagent_focus, None);
    assert_eq!(scroll, 7);
    assert!(!follow);
}

/// Clicking the row itself keeps its ctx-switch meaning: it enters the
/// subagent view and leaves the group fold alone.
#[tokio::test]
async fn clicking_a_visible_row_enters_the_view_without_toggling_the_fold() {
    let (mut chat, hits, _, row_rect) = chat_and_hits(false);
    let row_rect = row_rect.expect("row target while expanded");
    let mut subagent_focus: Option<usize> = None;
    let mut scroll = 3u32;
    let mut follow = false;
    let outcome = click_at(
        &hits,
        &mut chat,
        &mut subagent_focus,
        &mut scroll,
        &mut follow,
        row_rect.x + 1, // the leading columns are NOT a fold target anymore
        row_rect.y,
    )
    .await;
    assert_eq!(outcome, MouseOutcome::None);
    assert_eq!(
        subagent_focus,
        Some(0),
        "clicking the row must enter the subagent view"
    );
    assert_eq!(scroll, 0, "entering resets scroll");
    assert!(follow, "entering enables follow mode");
    assert!(
        !chat.subagents_collapsed,
        "entering the view must not collapse the parent's run"
    );
}

/// Collapsed: no `subagent_btns` exist, so a click on the hidden row's
/// former position does nothing — neither entering nor toggling — while the
/// `▸ subagents` header still toggles (re-expands).
#[tokio::test]
async fn collapsed_clicks_only_the_group_header_remains_actionable() {
    let (mut chat, hits, group_rect, _) = chat_and_hits(true);
    let hidden_row = Rect::new(1, group_rect.y + 1, 78, 1);
    let mut subagent_focus: Option<usize> = None;
    let mut scroll = 5u32;
    let mut follow = false;
    click_at(
        &hits,
        &mut chat,
        &mut subagent_focus,
        &mut scroll,
        &mut follow,
        hidden_row.x + 10,
        hidden_row.y,
    )
    .await;
    assert_eq!(subagent_focus, None, "hidden rows cannot be entered");
    assert!(
        chat.subagents_collapsed,
        "a hidden row is not a fold target"
    );
    assert_eq!(scroll, 5);

    click_at(
        &hits,
        &mut chat,
        &mut subagent_focus,
        &mut scroll,
        &mut follow,
        group_rect.x,
        group_rect.y,
    )
    .await;
    assert!(!chat.subagents_collapsed, "the header re-expands the run");
    assert_eq!(subagent_focus, None);
    assert_eq!(scroll, 5);
    assert!(!follow);
}

/// Inside a focused subagent, hits are computed against the CHILD view, so
/// the group header click toggles the child's own run (`collapse_view`
/// resolves the focused view) — the parent's fold is untouched and the
/// focus stays inside the child.
#[tokio::test]
async fn group_header_click_inside_a_focused_subagent_toggles_the_child_view() {
    let mut chat = ChatView::default();
    chat.apply(&SessionEvent::SubagentStart {
        id: "s1".into(),
        kind: "explore".into(),
        prompt: "outer".into(),
        child_session_id: "c1".into(),
    });
    // The child view runs a nested subagent of its own.
    chat.apply(&SessionEvent::SubagentChild {
        id: "s1".into(),
        ev: Box::new(SessionEvent::SubagentStart {
            id: "nested".into(),
            kind: "explore".into(),
            prompt: "inner".into(),
            child_session_id: "c-nested".into(),
        }),
    });
    let parent_idx = chat
        .blocks
        .iter()
        .position(|b| matches!(b, crate::chat::ChatBlock::Subagent { .. }))
        .expect("outer subagent block");
    let child_flags = |chat: &ChatView| -> (bool, bool) {
        let crate::chat::ChatBlock::Subagent { view, .. } = &chat.blocks[parent_idx] else {
            panic!("outer block");
        };
        (chat.subagents_collapsed, view.subagents_collapsed)
    };
    assert_eq!(child_flags(&chat), (false, false));
    // The child's nested run header is its flattened row 0 (child-relative).
    let body = Rect::new(0, 0, 80, 12);
    let mut hits = empty_hits(body);
    hits.subagent_group_btns.push(SubagentGroupBtn {
        rect: Rect::new(1, 1, 78, 1),
    });
    hits.subagent_btns.push(SubagentBtn {
        block_idx: 0,
        rect: Rect::new(1, 2, 78, 1),
    });

    let mut subagent_focus = Some(parent_idx);
    let mut scroll = 0u32;
    let mut follow = true;
    let outcome = click_at(
        &hits,
        &mut chat,
        &mut subagent_focus,
        &mut scroll,
        &mut follow,
        1,
        1,
    )
    .await;
    assert_eq!(outcome, MouseOutcome::None);
    assert_eq!(
        subagent_focus,
        Some(parent_idx),
        "the group header click must stay inside the focused child view"
    );
    assert_eq!(
        child_flags(&chat),
        (false, true),
        "only the CHILD view's run folds"
    );
    assert_eq!(scroll, 0);
    assert!(follow);
}
