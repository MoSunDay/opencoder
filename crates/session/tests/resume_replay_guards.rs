//! Duplicate/orphan `tool_result` guards for `resume_and_replay`
//! (cross-process resume). A non-terminal task whose `tool_use` id already
//! carries a persisted `tool_result`, or whose dispatch message sits below a
//! handoff/compaction boundary, must NOT be replayed — the backfill would
//! append a duplicate (or orphan) `tool_result` that OpenAI-compatible
//! providers reject with HTTP 400, permanently breaking the session.

use std::path::PathBuf;
use std::sync::Arc;

use opencoder_core::{ContentBlock, Message, MessageUsage, Role};
use opencoder_llm::{ChatStream, MockChatClient};
use opencoder_session::resume_and_replay;
use opencoder_store::{SessionPatch, SubagentStatus, SubagentTaskRecord};

mod common;
use common::*;

/// A tool-role message carrying exactly one `tool_result` (e.g. the terminal
/// "timed out" result the subagent timeout path persists).
fn tool_result_msg(id: &str, content: &str, is_error: bool) -> Message {
    Message {
        id: format!("{id}-result"),
        role: Role::Tool,
        blocks: vec![ContentBlock::ToolResult {
            tool_use_id: id.into(),
            content: content.into(),
            is_error,
            images: Vec::new(),
        }],
        model: None,
        agent: None,
        usage: MessageUsage::default(),
        created_at: 0,
        synthetic: false,
    }
}

fn task_row(task_id: &str, child: &str, status: SubagentStatus) -> SubagentTaskRecord {
    SubagentTaskRecord {
        task_id: task_id.into(),
        parent_session_id: "parent".into(),
        child_session_id: child.into(),
        parent_message_id: Some("a1".into()),
        agent: "explore".into(),
        prompt: "explore the codebase".into(),
        result: None,
        status,
        ok: None,
        started_at: 0,
        completed_at: None,
    }
}

/// Multiset of `tool_use` ids vs `tool_result` ids: well-formed iff every
/// `tool_use` is answered exactly once and no `tool_result` is an orphan or
/// a duplicate.
fn pairing_counts(msgs: &[Message]) -> (Vec<String>, Vec<String>) {
    let mut uses = Vec::new();
    let mut results = Vec::new();
    for m in msgs {
        for b in &m.blocks {
            match b {
                ContentBlock::ToolUse { id, .. } => uses.push(id.clone()),
                ContentBlock::ToolResult { tool_use_id, .. } => results.push(tool_use_id.clone()),
                _ => {}
            }
        }
    }
    (uses, results)
}

fn assert_pairing_well_formed(msgs: &[Message]) {
    let (mut uses, mut results) = pairing_counts(msgs);
    uses.sort();
    results.sort();
    assert_eq!(
        uses, results,
        "every tool_use must be answered exactly once (no dangling, orphan, or duplicate ids)"
    );
}

fn count_results_for(msgs: &[Message], id: &str) -> usize {
    msgs.iter()
        .flat_map(|m| m.blocks.iter())
        .filter(|b| {
            matches!(b, ContentBlock::ToolResult { tool_use_id, .. } if tool_use_id == id)
        })
        .count()
}

async fn seed_parent_with_child(store: &Arc<dyn opencoder_store::Store>, child: &str) {
    store
        .create_session(&session_meta("parent", "act"))
        .await
        .unwrap();
    store
        .create_session(&session_meta(child, "explore"))
        .await
        .unwrap();
    store
        .append_message("parent", &Message::user("u1", "please explore"))
        .await
        .unwrap();
    // Child transcript: the original dispatch prompt (history to resume from).
    store
        .append_message(child, &Message::user("cu1", "explore the codebase"))
        .await
        .unwrap();
}

/// Duplicate guard: a task whose `tool_result` is already persisted (timeout
/// path recorded it; a crash left the row non-terminal) must be skipped —
/// replaying would append a second `tool_result` for one `tool_use` id.
#[tokio::test]
async fn resume_and_replay_skips_task_with_persisted_result() {
    let store = mem_store().await;
    seed_parent_with_child(&store, "child-1").await;
    store
        .append_message("parent", &parent_task_turn(&["task-dup"]))
        .await
        .unwrap();
    store
        .append_message("parent", &tool_result_msg("task-dup", "subagent timed out", true))
        .await
        .unwrap();
    store
        .create_subagent_task(&task_row("task-dup", "child-1", SubagentStatus::Running))
        .await
        .unwrap();

    let mock = Arc::new(MockChatClient::new());
    let session = resume_and_replay(
        store.clone(),
        "parent",
        config("m"),
        mock.clone() as Arc<dyn ChatStream>,
        PathBuf::from("/tmp"),
        None,
    )
    .await
    .unwrap();

    // The child was never resumed: no LLM call, no new parent messages.
    assert_eq!(mock.call_count(), 0, "answered task must not be replayed");
    let msgs = store.load_messages("parent").await.unwrap();
    assert_eq!(
        msgs.len(),
        3,
        "no duplicate tool_result may be appended for an answered task"
    );
    assert_eq!(count_results_for(&msgs, "task-dup"), 1);

    // Rebuilt transcript stays provider-well-formed: exactly one result per
    // tool_use, no orphans, no duplicates.
    assert_pairing_well_formed(&session.messages);

    // Skipped row left untouched by the guard: it must NOT look Completed
    // (nothing was replayed). A `Running` row is reconciled to `Cancelled` by
    // `resume`; any later resume re-filters it the same way (idempotent).
    let row = store.get_subagent_task("task-dup").await.unwrap().unwrap();
    assert_ne!(
        row.status,
        SubagentStatus::Completed,
        "skipped task must not be marked Completed"
    );
}

/// Boundary guard (handoff): the dispatch assistant message sits below
/// `handoff_seq`, so `resume` will never show the task's `tool_use`. A
/// backfilled result would be an orphan — it must not be appended.
#[tokio::test]
async fn resume_and_replay_skips_task_dispatched_below_handoff_boundary() {
    let store = mem_store().await;
    seed_parent_with_child(&store, "child-1").await;
    store
        .append_message("parent", &parent_task_turn(&["task-below"]))
        .await
        .unwrap();
    // One act-mode message above the boundary keeps the visible tail alive.
    store
        .append_message("parent", &Message::user("u2", "post-handoff"))
        .await
        .unwrap();
    // Boundary ABOVE the dispatch message (2 pre-handoff store messages).
    store
        .update_session(
            "parent",
            &SessionPatch {
                handoff_seq: Some(2),
                handoff_plan: Some("## Plan\n1. do X".into()),
                updated_at: Some(0),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    store
        .create_subagent_task(&task_row("task-below", "child-1", SubagentStatus::Running))
        .await
        .unwrap();

    let mock = Arc::new(MockChatClient::new());
    let session = resume_and_replay(
        store.clone(),
        "parent",
        config("m"),
        mock.clone() as Arc<dyn ChatStream>,
        PathBuf::from("/tmp"),
        None,
    )
    .await
    .unwrap();

    assert_eq!(mock.call_count(), 0, "invisible task must not be replayed");
    let msgs = store.load_messages("parent").await.unwrap();
    assert_eq!(msgs.len(), 3, "no orphan tool_result may be backfilled");
    assert_eq!(
        count_results_for(&msgs, "task-below"),
        0,
        "no tool_result for a tool_use the provider cannot see"
    );
    // Rebuilt tail: [handoff head, post-handoff msg] — no orphan results.
    assert_eq!(session.messages.len(), 2);
    assert_pairing_well_formed(&session.messages);

    let row = store
        .get_subagent_task("task-below")
        .await
        .unwrap()
        .unwrap();
    assert_ne!(row.status, SubagentStatus::Completed);
}

/// Boundary guard (compaction): same as the handoff variant but with
/// `summary_seq`, exercising the `load_messages_after` tail path.
#[tokio::test]
async fn resume_and_replay_skips_task_dispatched_below_compaction_boundary() {
    let store = mem_store().await;
    seed_parent_with_child(&store, "child-1").await;
    store
        .append_message("parent", &parent_task_turn(&["task-compacted"]))
        .await
        .unwrap();
    store
        .append_message("parent", &Message::user("u2", "post-compaction"))
        .await
        .unwrap();
    store
        .update_session(
            "parent",
            &SessionPatch {
                summary_seq: Some(2),
                summary: Some("compacted head".into()),
                updated_at: Some(0),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    store
        .create_subagent_task(&task_row(
            "task-compacted",
            "child-1",
            SubagentStatus::Running,
        ))
        .await
        .unwrap();

    let mock = Arc::new(MockChatClient::new());
    let session = resume_and_replay(
        store.clone(),
        "parent",
        config("m"),
        mock.clone() as Arc<dyn ChatStream>,
        PathBuf::from("/tmp"),
        None,
    )
    .await
    .unwrap();

    assert_eq!(mock.call_count(), 0, "invisible task must not be replayed");
    let msgs = store.load_messages("parent").await.unwrap();
    assert_eq!(msgs.len(), 3, "no orphan tool_result may be backfilled");
    assert_eq!(count_results_for(&msgs, "task-compacted"), 0);
    // Rebuilt tail: [compaction summary, post-compaction msg] — well-formed.
    assert_eq!(session.messages.len(), 2);
    assert_pairing_well_formed(&session.messages);
}

/// The guards must not over-reach: in one parent holding one already-answered
/// task and one visible unanswered task, exactly the unanswered one is
/// replayed and backfilled (happy path preserved alongside the skip).
#[tokio::test]
async fn resume_and_replay_replays_only_the_unanswered_visible_task() {
    let store = mem_store().await;
    seed_parent_with_child(&store, "child-open").await;
    store
        .create_session(&session_meta("child-answered", "explore"))
        .await
        .unwrap();
    // One assistant turn dispatching BOTH tasks (ids differ, message id a1).
    store
        .append_message("parent", &parent_task_turn(&["task-answered", "task-open"]))
        .await
        .unwrap();
    store
        .append_message("parent", &tool_result_msg("task-answered", "timed out earlier", true))
        .await
        .unwrap();
    store
        .create_session(&session_meta("child-answered", "explore"))
        .await
        .unwrap();
    store
        .append_message(
            "child-answered",
            &Message::user("cu1", "explore the codebase"),
        )
        .await
        .unwrap();
    store
        .create_subagent_task(&task_row(
            "task-answered",
            "child-answered",
            SubagentStatus::Cancelled,
        ))
        .await
        .unwrap();
    store
        .create_subagent_task(&task_row("task-open", "child-open", SubagentStatus::Running))
        .await
        .unwrap();

    let mock = Arc::new(MockChatClient::new().push_script(vec![done_event("found 3 files")]));
    let session = resume_and_replay(
        store.clone(),
        "parent",
        config("m"),
        mock.clone() as Arc<dyn ChatStream>,
        PathBuf::from("/tmp"),
        None,
    )
    .await
    .unwrap();

    // Exactly one replay — the visible, unanswered task.
    assert_eq!(mock.call_count(), 1);
    let msgs = store.load_messages("parent").await.unwrap();
    assert_eq!(count_results_for(&msgs, "task-answered"), 1, "no duplicate");
    assert_eq!(count_results_for(&msgs, "task-open"), 1, "backfilled once");
    assert_pairing_well_formed(&session.messages);

    // Terminal states: only the replayed task completed.
    let open = store.get_subagent_task("task-open").await.unwrap().unwrap();
    assert!(matches!(open.status, SubagentStatus::Completed));
    let answered = store
        .get_subagent_task("task-answered")
        .await
        .unwrap()
        .unwrap();
    assert_ne!(answered.status, SubagentStatus::Completed);

    // The untouched child was never resumed.
    let child_answered = store.load_messages("child-answered").await.unwrap();
    assert_eq!(child_answered.len(), 1, "answered task's child untouched");
}

/// Sanity check of the multiset counts themselves (guard against a vacuous
/// test helper): duplicated and orphaned ids must be detectable.
#[test]
fn pairing_helper_detects_duplicates_and_orphans() {
    let dup = vec![
        parent_task_turn(&["t1"]),
        tool_result_msg("t1", "one", false),
        tool_result_msg("t1", "two", false),
    ];
    let (uses, results) = pairing_counts(&dup);
    assert_eq!(uses.len(), 1, "one tool_use");
    assert_eq!(results.len(), 2, "duplicated results");

    let orphan = vec![tool_result_msg("ghost", "orphan", false)];
    let (uses, results) = pairing_counts(&orphan);
    assert!(uses.is_empty());
    assert_eq!(results.len(), 1, "orphan result with no tool_use");
}
