//! Grandchild `Subagent` cards exist only in the nested view being folded
//! (no store handle inside `ChatView::apply`), so the store-free reset fold
//! (`reset_fold::rebuild_view_from_messages`) carries them across instead of
//! rebuilding them from records — both child paths: the live
//! `SubagentChild(TranscriptReset)` routing and the persisted-event replay.

use super::replay::build_subagent_block;
use super::subagent_block_tests::{task_record, EmptyChildStore};
use super::subagent_compaction_tests::{child_view, flattened, summary_message};
use crate::chat::{ChatBlock, ChatView};
use opencoder_core::{ContentBlock, Message};
use opencoder_session::SessionEvent;
use opencoder_store::SubagentStatus;
use std::sync::Arc;

/// The assistant message that spawned subagent task `task_id`: its `task`
/// tool_use carries the task id (the identity `replay_one` keys on).
fn spawning_message(id: &str, task_id: &str) -> Message {
    let mut m = Message::assistant(id);
    m.blocks.push(ContentBlock::ToolUse {
        id: task_id.into(),
        name: "task".into(),
        input: serde_json::json!({ "prompt": "spawn" }),
    });
    m
}

fn child_event(id: &str, ev: SessionEvent) -> SessionEvent {
    SessionEvent::SubagentChild {
        id: id.into(),
        ev: Box::new(ev),
    }
}

/// A parent view whose child `s1` already dispatched grandchild `g1`.
fn parent_with_running_grandchild() -> ChatView {
    let mut parent = ChatView::default();
    parent.apply(&SessionEvent::SubagentStart {
        id: "s1".into(),
        kind: "explore".into(),
        prompt: "find x".into(),
        child_session_id: "c1".into(),
    });
    parent.apply(&child_event(
        "s1",
        SessionEvent::SubagentStart {
            id: "g1".into(),
            kind: "explore".into(),
            prompt: "scan".into(),
            child_session_id: "c1-g1".into(),
        },
    ));
    parent.apply(&child_event(
        "s1",
        SessionEvent::TextDelta("old work".into()),
    ));
    parent
}

fn subagent_card_positions(view: &ChatView) -> Vec<usize> {
    view.blocks
        .iter()
        .enumerate()
        .filter(|(_, b)| matches!(b, ChatBlock::Subagent { .. }))
        .map(|(i, _)| i)
        .collect()
}

/// Grandchild cards live only in the child view (no store in `apply`): the
/// live nested reset re-attaches them right under the assistant message that
/// spawned them, the counters stay consistent with the visible cards, and a
/// later grandchild event still routes into the card.
#[test]
fn child_live_reset_reattaches_grandchild_cards_under_their_parent_message() {
    let mut parent = parent_with_running_grandchild();
    let folded = vec![
        summary_message("s1", "folded history"),
        spawning_message("a1", "g1"),
        Message::user("u2", "tail prompt"),
    ];
    parent.apply(&child_event("s1", SessionEvent::TranscriptReset(folded)));

    let view = child_view(&parent.blocks[0]);
    assert!(
        matches!(view.blocks.first(), Some(ChatBlock::Compaction { .. })),
        "summary stays the head"
    );
    let cards = subagent_card_positions(view);
    assert_eq!(cards.len(), 1, "the grandchild card survives the fold");
    let tail = view
        .blocks
        .iter()
        .position(|b| matches!(b, ChatBlock::User { .. }))
        .expect("tail prompt replayed");
    assert!(
        cards[0] > 0 && cards[0] < tail,
        "card under its parent message"
    );
    assert_eq!(view.subagents_total, 1);
    assert_eq!(view.subagents_running, 1);
    assert!(!flattened(view).contains("old work"));

    parent.apply(&child_event(
        "s1",
        child_event("g1", SessionEvent::TextDelta("grandchild work".into())),
    ));
    let view = child_view(&parent.blocks[0]);
    assert!(
        flattened(child_view(&view.blocks[cards[0]])).contains("grandchild work"),
        "routing into the re-attached card keeps working"
    );
}

/// A grandchild whose spawning message was folded away keeps its card — the
/// store replay would drop the record — appended like an orphan task, so the
/// counters and the visible cards stay consistent across the nested reset.
#[test]
fn child_live_reset_keeps_grandchild_cards_whose_spawning_message_was_folded() {
    let mut parent = parent_with_running_grandchild();
    let folded = vec![
        summary_message("s1", "folded history"),
        Message::user("u2", "tail prompt"),
    ];
    parent.apply(&child_event("s1", SessionEvent::TranscriptReset(folded)));

    let view = child_view(&parent.blocks[0]);
    assert!(matches!(
        view.blocks.first(),
        Some(ChatBlock::Compaction { .. })
    ));
    let cards = subagent_card_positions(view);
    assert_eq!(cards.len(), 1, "the live card is kept, not dropped");
    assert_eq!(
        cards[0],
        view.blocks.len() - 1,
        "appended after the retained tail like an orphan task"
    );
    assert_eq!(view.subagents_total as usize, cards.len());
    assert_eq!(view.subagents_running, 1);
}

/// Persisted child log (`/task` switch, resume, parent rebuild): the replayed
/// `SubagentStart` card survives the replayed `TranscriptReset` the same way.
#[tokio::test]
async fn child_replay_keeps_grandchild_cards_across_the_folded_reset() {
    let folded = vec![
        summary_message("s1", "folded history"),
        spawning_message("a1", "g1"),
        Message::user("u2", "tail prompt"),
    ];
    let events = vec![
        SessionEvent::SubagentStart {
            id: "g1".into(),
            kind: "explore".into(),
            prompt: "scan".into(),
            child_session_id: "c1-g1".into(),
        },
        SessionEvent::TextDelta("old work".into()),
        SessionEvent::TranscriptReset(folded),
        SessionEvent::TextDelta("continued work".into()),
    ];
    let store: Arc<dyn opencoder_store::Store> = Arc::new(EmptyChildStore::with_events(events));
    let task = task_record(SubagentStatus::Completed, Some("done"), Some(true));
    let block = build_subagent_block(&task, &store).await;
    let view = child_view(&block);

    assert!(
        matches!(view.blocks.first(), Some(ChatBlock::Compaction { .. })),
        "summary stays the head"
    );
    assert_eq!(
        subagent_card_positions(view).len(),
        1,
        "grandchild card survives the replayed reset"
    );
    assert_eq!(view.subagents_total, 1);
    let text = flattened(view);
    assert!(!text.contains("old work"));
    assert!(text.contains("tail prompt"));
    assert!(text.contains("continued work"));
}
