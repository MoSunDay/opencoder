//! Plan-card dedup: a replayed/duplicate `PlanHandoff` event must not stack a
//! second plan card on top of the first. On resume or re-delivery the display
//! layer already holds a Plan block, so subsequent handoffs are ignored.

use opencoder_session::SessionEvent;
use opencoder_tui::chat::{ChatBlock, ChatView};

#[test]
fn plan_handoff_skips_duplicate_plan_card() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::PlanHandoff("## Plan\n1. do X".into()));
    v.apply(&SessionEvent::PlanHandoff("## Plan\n1. do Y".into()));

    let plan_count = v
        .blocks
        .iter()
        .filter(|b| matches!(b, ChatBlock::Plan { .. }))
        .count();
    assert_eq!(
        plan_count, 1,
        "a duplicate PlanHandoff must not create a second plan card"
    );
}
