//! Subagent child views must render compaction exactly like the parent
//! transcript: ONE collapsed `Compaction` block at the head, followed by the
//! retained tail and the continued run — never the child's whole
//! pre-compaction history plus one summary block per compaction.
//!
//! Both child paths are covered: the persisted-event replay
//! (`reconstruct_child_view`, used by resume / `/task` switch / parent
//! rebuild) and the live `SubagentChild` routing inside `ChatView::apply`.

use super::replay::build_subagent_block;
use super::subagent_block_tests::{task_record, EmptyChildStore};
use crate::chat::{ChatBlock, ChatView};
use opencoder_core::Message;
use opencoder_session::SessionEvent;
use opencoder_store::SubagentStatus;
use std::sync::Arc;

fn summary_message(id: &str, summary: &str) -> Message {
    // Shape mirrors `opencoder_session::compaction::compaction_message`.
    let mut m = Message::user(
        id,
        format!(
            "{}\n{summary}",
            opencoder_session::compaction::SUMMARY_PREFIX
        ),
    );
    m.synthetic = true;
    m
}

fn compaction_blocks(view: &ChatView) -> Vec<&ChatBlock> {
    view.blocks
        .iter()
        .filter(|b| matches!(b, ChatBlock::Compaction { .. }))
        .collect()
}

fn child_view(block: &ChatBlock) -> &ChatView {
    match block {
        ChatBlock::Subagent { view, .. } => view,
        other => panic!("expected Subagent block, got {other:?}"),
    }
}

fn flattened(view: &ChatView) -> String {
    view.flatten()
        .iter()
        .map(|l| l.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

/// One persisted child compaction: the pre-compaction blocks are folded away
/// and the summary renders first, exactly like the parent's rebuild.
#[tokio::test]
async fn child_replay_folds_history_into_one_head_summary() {
    let folded = vec![
        summary_message("s1", "folded history"),
        Message::user("u2", "tail prompt"),
    ];
    let events = vec![
        SessionEvent::TextDelta("old work".into()),
        SessionEvent::CompactionDelta("folded hist".into()),
        SessionEvent::TranscriptReset(folded),
        SessionEvent::Compaction("folded history".into()),
        SessionEvent::TextDelta("continued work".into()),
    ];
    let store: Arc<dyn opencoder_store::Store> = Arc::new(EmptyChildStore::with_events(events));
    let task = task_record(SubagentStatus::Completed, Some("done"), Some(true));
    let block = build_subagent_block(&task, &store).await;
    let view = child_view(&block);

    let compactions = compaction_blocks(view);
    assert_eq!(
        compactions.len(),
        1,
        "streamed block + replay twin + event must stay one block"
    );
    assert!(
        matches!(view.blocks.first(), Some(ChatBlock::Compaction { .. })),
        "summary must be the FIRST block, got {:?}",
        view.blocks.first()
    );
    assert!(
        matches!(
            compactions[0],
            ChatBlock::Compaction {
                collapsed: true,
                streaming: false,
                ..
            }
        ),
        "replay twin starts collapsed like the parent's"
    );
    let text = flattened(view);
    assert!(
        !text.contains("old work"),
        "pre-compaction history folded away"
    );
    assert!(text.contains("tail prompt"));
    assert!(text.contains("continued work"));
}

/// A child that compacted TWICE still shows one summary block — the latest
/// — at the head. The event log carries every streaming block and every
/// summary; only the final folded transcript may survive.
#[tokio::test]
async fn child_replay_two_compactions_keep_only_the_latest_summary() {
    let first = vec![summary_message("s1", "first summary")];
    let second = vec![
        summary_message("s2", "second summary"),
        Message::user("u9", "late tail"),
    ];
    let events = vec![
        SessionEvent::CompactionDelta("first su".into()),
        SessionEvent::TranscriptReset(first),
        SessionEvent::Compaction("first summary".into()),
        SessionEvent::TextDelta("between".into()),
        SessionEvent::TranscriptReset(second),
        SessionEvent::Compaction("second summary".into()),
    ];
    let store: Arc<dyn opencoder_store::Store> = Arc::new(EmptyChildStore::with_events(events));
    let task = task_record(SubagentStatus::Completed, Some("done"), Some(true));
    let block = build_subagent_block(&task, &store).await;
    let view = child_view(&block);

    let compactions = compaction_blocks(view);
    assert_eq!(compactions.len(), 1, "only the latest summary survives");
    assert!(matches!(
        view.blocks.first(),
        Some(ChatBlock::Compaction { .. })
    ));
    let text = flattened(view);
    assert!(
        text.contains("Compaction (1 lines)"),
        "collapsed header at the transcript head: {text}"
    );
    assert!(
        !text.contains("between"),
        "pre-second-compaction run folded away"
    );
    assert!(text.contains("late tail"));
    // Expanding reveals the LATEST summary text, never the first one.
    let mut expanded = view.clone();
    expanded.toggle_compaction_at(0);
    let text = flattened(&expanded);
    assert!(text.contains("second summary"));
    assert!(!text.contains("first summary"));
}

/// The LIVE child path: `SubagentChild(TranscriptReset)` rebuilds the nested
/// view in place, so the following `Compaction` event dedups against the
/// replay twin instead of stacking a second block mid-transcript.
#[test]
fn child_live_reset_folds_the_nested_view_like_the_parent() {
    let mut parent = ChatView::default();
    parent.apply(&SessionEvent::SubagentStart {
        id: "s1".into(),
        kind: "explore".into(),
        prompt: "find x".into(),
        child_session_id: "c1".into(),
    });
    let child = |ev: SessionEvent| SessionEvent::SubagentChild {
        id: "s1".into(),
        ev: Box::new(ev),
    };
    parent.apply(&child(SessionEvent::TextDelta("old work".into())));
    parent.apply(&child(SessionEvent::CompactionDelta("folded".into())));

    let folded = vec![
        summary_message("s1", "folded history"),
        Message::user("u2", "tail prompt"),
    ];
    parent.apply(&child(SessionEvent::TranscriptReset(folded)));
    parent.apply(&child(SessionEvent::Compaction("folded history".into())));
    parent.apply(&child(SessionEvent::TextDelta("continued work".into())));

    let Some(ChatBlock::Subagent { view, .. }) = parent.blocks.first() else {
        panic!("expected the subagent block");
    };
    let compactions = compaction_blocks(view);
    assert_eq!(
        compactions.len(),
        1,
        "one summary block after the live reset"
    );
    assert!(
        matches!(view.blocks.first(), Some(ChatBlock::Compaction { .. })),
        "summary is the child transcript's head"
    );
    let text = flattened(view);
    assert!(!text.contains("old work"));
    assert!(text.contains("tail prompt"));
    assert!(text.contains("continued work"));
    // Lifetime cost survives the nested rebuild.
    assert_eq!(parent.subagents_running, 1);
}

/// Cross-reset carry-over of the store-free rebuild: lifetime tokens are
/// floored, the first prompt and grandchild counters survive, and a
/// compacting rebuild drops the in-flight echo exactly like the parent's
/// off-loop rebuild.
#[test]
fn store_free_rebuild_carries_lifetime_state_but_drops_compacting_echo() {
    let mut view = ChatView {
        tokens_total: 500,
        subagents_running: 2,
        subagents_total: 3,
        first_prompt: Some("original task".into()),
        pending_turn_echo: Some("in-flight prompt".into()),
        ..Default::default()
    };
    let saved_first_prompt = view.first_prompt.clone();

    super::rebuild_view_from_messages(&mut view, &[summary_message("s1", "folded history")]);

    assert_eq!(view.tokens_total, 500, "tok cost never regresses");
    assert_eq!(view.subagents_running, 2);
    assert_eq!(view.subagents_total, 3);
    assert_eq!(view.pending_turn_echo, None, "compaction drops the echo");
    assert_eq!(view.first_prompt, saved_first_prompt);
    assert_eq!(view.turn_block_start, view.blocks.len());
}

/// clear-context reset (no summary in the folded list): no Compaction block
/// appears and the in-flight echo survives, mirroring the parent's rebuild.
#[test]
fn store_free_rebuild_keeps_echo_when_reset_is_not_a_compaction() {
    let mut view = ChatView {
        pending_turn_echo: Some("in-flight prompt".into()),
        ..Default::default()
    };
    super::rebuild_view_from_messages(&mut view, &[Message::user("u1", "retained tail")]);
    assert!(compaction_blocks(&view).is_empty(), "no summary, no block");
    assert_eq!(view.pending_turn_echo.as_deref(), Some("in-flight prompt"));
    // Same rule as the parent: the saved (unset) first prompt wins — the
    // retained tail prompt is never promoted to `first_prompt`.
    assert_eq!(view.first_prompt, None);
    assert_eq!(view.turn_block_start, view.blocks.len());
}
