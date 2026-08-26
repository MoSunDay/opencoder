//! Body wheel-scroll tests for `handle_mouse` (`ScrollDown` / `ScrollUp`):
//! subagent-aware content sizing and the 8-line wheel-up step. Split from
//! `mouse_tests.rs` to keep files within the iteration caps.

use super::mouse_helpers::*;
use crate::app_helpers::*;
use crossterm::event::KeyModifiers;
use ratatui::layout::Rect;

fn scroll_down() -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: 40,
        row: 6,
        modifiers: KeyModifiers::NONE,
    }
}
/// The regression: with a subagent focused, one wheel-down must NOT pin to
/// the bottom even though the PARENT fits in the viewport (which, under the
/// old parent-based `max_rows`, saturated to 0 and tripped `follow`).
#[tokio::test]
async fn scrolldown_in_subagent_view_uses_child_content() {
    let mut chat = parent_with_long_subagent();
    let sub_idx = chat
        .blocks
        .iter()
        .position(|b| matches!(b, crate::chat::ChatBlock::Subagent { .. }))
        .expect("a Subagent block exists");

    let parent_rows = chat.flatten().len();
    let child_rows = match &chat.blocks[sub_idx] {
        crate::chat::ChatBlock::Subagent { view, .. } => view.flatten().len(),
        _ => unreachable!(),
    };
    let body = Rect::new(0, 0, 80, 12); // visible_h = 10, inner_w = 77
    let visible_h = body.height as usize - 2;
    assert!(
            child_rows > parent_rows && child_rows > visible_h,
            "precondition: child ({child_rows}) longer than parent ({parent_rows}) and viewport ({visible_h})"
        );
    // Parent must fit in the viewport — that is what made the old math trip.
    assert!(
        parent_rows < visible_h,
        "precondition: parent ({parent_rows}) fits viewport ({visible_h})"
    );

    let mut hits = empty_hits(body);
    // render_body caches the viewed content's row count in total_rows; when a
    // subagent is focused that is the child view. Mirror it here so the
    // scroll-wheel clamp sees real child content below the fold.
    hits.total_rows = child_rows;
    let mut scroll = 0u32;
    let mut follow = false;
    let mut subagent_focus = Some(sub_idx);
    let mut subagent_sys = 0u64;
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let store = StubStore;
    let mut queue_scroll: u32 = 0;

    handle_mouse(
        scroll_down(),
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

    assert_eq!(scroll, 3, "scroll advanced by one notch");
    assert!(
        !follow,
        "follow must NOT trip: the child still has content below the fold"
    );
}

/// Mirror case: with NO subagent focused, the parent view drives `max_rows`.
/// Here the short parent fits the viewport, so the first wheel-down
/// legitimately pins to the bottom.
#[tokio::test]
async fn scrolldown_uses_parent_when_no_subagent_focused() {
    let mut chat = parent_with_long_subagent();
    let body = Rect::new(0, 0, 80, 12);
    let visible_h = body.height as usize - 2;
    assert!(
        chat.flatten().len() < visible_h,
        "precondition: parent fits viewport"
    );

    let hits = empty_hits(body);
    let mut scroll = 0u32;
    let mut follow = false;
    let mut subagent_focus: Option<usize> = None;
    let mut subagent_sys = 0u64;
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let store = StubStore;
    let mut queue_scroll: u32 = 0;

    handle_mouse(
        scroll_down(),
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
        "short parent legitimately pins to bottom immediately"
    );
}

/// Wheel-up now advances 8 lines per notch (was 3) so scrolling back up
/// through a long transcript feels responsive. Down is unchanged at 3.
#[tokio::test]
async fn scrollup_advances_faster_than_default() {
    // Build a long-enough ChatView so content clearly exceeds the small
    // viewport (visible_h = body.height - 2 = 10).
    let mut chat = ChatView::default();
    for n in 0..30u32 {
        chat.push_marker(Line::from(format!("marker line {n}")));
    }

    let body = Rect::new(0, 0, 80, 12);
    let hits = empty_hits(body);

    let scroll_up = || MouseEvent {
        kind: MouseEventKind::ScrollUp,
        column: 40,
        row: 6,
        modifiers: KeyModifiers::NONE,
    };

    // `scroll` is the top-anchored line offset (0 == top); scroll-up moves
    // toward the top via `saturating_sub`. Start part-way down so a single
    // notch lands on a value that proves the 8-line step: the new step
    // yields 16 - 8 = 8, whereas the old 3-step would have left 16 - 3 = 13.
    let mut scroll = 16u32;
    let mut follow = true;
    let mut subagent_focus: Option<usize> = None;
    let mut subagent_sys = 0u64;
    let mut queue_items: Vec<(i64, String)> = vec![];
    let mut queue_scroll: u32 = 0;
    let store = StubStore;

    handle_mouse(
        scroll_up(),
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

    assert_eq!(scroll, 8, "one wheel-up notch now moves 8 lines (was 3)");
    assert!(!follow, "scrolling up must detach from the tail");
}
