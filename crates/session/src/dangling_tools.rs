//! Well-formedness safety net for tool-call transcripts.
//!
//! The OpenAI-compatible chat API requires every assistant `tool_use` id to be
//! answered by a matching `tool_result`; a transcript with an unanswered id
//! (e.g. after a hard interrupt mid-tool-batch) is rejected with HTTP 400 on
//! the next request. This module is the single home for the dangling-tool_use
//! reconciliation logic shared by:
//!
//! - `resume::resume` (new-process resume, `session resume` / `--continue`):
//!   synthesizes error results for dangling ids before the session is rebuilt.
//! - `runner::run_with_registry` (in-process continuation: web drain, TUI
//!   double-Esc then continue, CLI retry): `reconcile_dangling_tool_uses`
//!   runs right after `replay_cancelled_tasks`, so history stays well-formed
//!   even when the hard-cancel branch in `run_loop` had to drop a tool batch.
//!
//! `task` tool_use ids whose subagent is still replayable (`Running` /
//! `Cancelled`) are deliberately excluded: their results are backfilled by
//! `replay_cancelled_tasks` / `resume_and_replay` on the next user turn, and
//! synthesizing an error result would permanently answer an id that replay
//! depends on staying open.
//!
//! The pure pairing predicates here (`tool_use_ids` / `tool_result_ids` /
//! `tool_use_ids_without_result`) are also reused by the autopilot VERIFY
//! snapshot repair (`autopilot::verify`), which must fix the same
//! use/result pairing defects at a window boundary — one shared definition.

use std::collections::HashSet;

use opencoder_core::{message::now_ms, ContentBlock, Message, MessageUsage, Role};
use opencoder_store::{SessionMeta, Store, SubagentStatus, SubagentTaskRecord};

use crate::SessionState;

/// Error text used for every synthesized `tool_result` (matches `resume.rs`).
pub const DANGLING_RESULT_MSG: &str = "session interrupted: tool result missing";

/// Ids of subagent tasks that are still replayable: `Running` (the child may
/// still finish or will be resumed by `resume_and_replay`) or `Cancelled`
/// (replayed / abandoned on the next user turn by `replay_cancelled_tasks`).
/// Their `task` tool_use ids stay dangling on purpose.
pub fn replayable_task_ids_from_records(records: &[SubagentTaskRecord]) -> HashSet<String> {
    records
        .iter()
        .filter(|t| {
            matches!(
                t.status,
                SubagentStatus::Running | SubagentStatus::Cancelled
            )
        })
        .map(|t| t.task_id.clone())
        .collect()
}

/// Ids of every `tool_use` block in `messages` (duplicates collapsed). Pure.
///
/// Shared pairing algebra: `dangling_tool_use_results` (below) and the
/// autopilot VERIFY snapshot repair (`autopilot::verify::build_snapshot`)
/// both need to ask "which tool_use ids exist in this transcript slice?" —
/// one definition keeps the two from drifting apart.
pub fn tool_use_ids(messages: &[Message]) -> HashSet<String> {
    messages
        .iter()
        .flat_map(|m| m.blocks.iter())
        .filter_map(|b| match b {
            ContentBlock::ToolUse { id, .. } => Some(id.clone()),
            _ => None,
        })
        .collect()
}

/// Ids of every `tool_result` block in `messages` (duplicates collapsed).
/// Pure companion of [`tool_use_ids`].
pub fn tool_result_ids(messages: &[Message]) -> HashSet<String> {
    messages
        .iter()
        .flat_map(|m| m.blocks.iter())
        .filter_map(|b| match b {
            ContentBlock::ToolResult { tool_use_id, .. } => Some(tool_use_id.clone()),
            _ => None,
        })
        .collect()
}

/// Ids of every `tool_use` in `messages` that has no matching `tool_result`
/// anywhere in `messages` — the dangling ids an OpenAI-compatible provider
/// rejects with HTTP 400 if the slice were sent as-is. Pure.
pub fn tool_use_ids_without_result(messages: &[Message]) -> HashSet<String> {
    let answered = tool_result_ids(messages);
    tool_use_ids(messages)
        .into_iter()
        .filter(|id| !answered.contains(id))
        .collect()
}

/// True when the assistant message that dispatched `task`'s `tool_use` is
/// among `visible` — i.e. the provider will actually see the `tool_use` that
/// a backfilled `tool_result` would answer. Prefers the recorded
/// `parent_message_id` (matched against visible message ids); falls back to
/// scanning `visible` for a `tool_use` block carrying the task id when the
/// link was not recorded. Pure; used by `resume::resume_and_replay`'s
/// orphan-result guard at handoff/compaction boundaries.
pub fn task_tool_use_visible(task: &SubagentTaskRecord, visible: &[Message]) -> bool {
    match task.parent_message_id.as_deref() {
        Some(parent_id) => visible.iter().any(|m| m.id == parent_id),
        None => visible.iter().any(|m| {
            m.blocks
                .iter()
                .any(|b| matches!(b, ContentBlock::ToolUse { id, .. } if id == &task.task_id))
        }),
    }
}

/// The parent transcript slice `resume::resume` will actually load: below a
/// plan→act handoff boundary (`handoff_seq` + `handoff_plan`) or a compaction
/// boundary (`summary_seq`), else the full transcript. Mirrors the
/// loading/trimming branches of `resume::resume`; the synthetic head message
/// it re-attaches never carries a `tool_use`, so it is irrelevant for
/// visibility. Used by the orphan half of `resume`'s replay guard to decide
/// whether a backfilled `tool_result` would answer a `tool_use` the provider
/// can see. Read failures degrade to an empty tail — conservative (every
/// candidate is dropped) and safe: no replay means no corruption, and
/// `resume` re-raises the store error.
pub async fn visible_parent_tail(
    store: &std::sync::Arc<dyn Store>,
    id: &str,
    meta: &SessionMeta,
) -> Vec<Message> {
    let messages =
        if meta.handoff_seq.is_none() && matches!(meta.summary_seq, Some(sk) if sk > 0) {
            store
                .load_messages_after(id, meta.summary_seq.unwrap())
                .await
        } else {
            store.load_messages(id).await
        }
        .unwrap_or_default();
    match (meta.handoff_seq, meta.handoff_plan.is_some()) {
        (Some(hs), true) => {
            let hs = hs as usize;
            if hs < messages.len() {
                messages[hs..].to_vec()
            } else {
                Vec::new()
            }
        }
        _ => messages,
    }
}

/// Compute synthetic error `ToolResult` blocks for every `tool_use` id in
/// `messages` that has no matching `tool_result` and is not in `replayable`.
/// Pure: no persistence, no mutation — callers decide what to do with the
/// result. Preserves transcript order. Built on
/// [`tool_use_ids_without_result`]; keeps per-block iteration so duplicate
/// dangling ids still yield one result block each.
pub fn dangling_tool_use_results(
    messages: &[Message],
    replayable: &HashSet<String>,
) -> Vec<ContentBlock> {
    let dangling = tool_use_ids_without_result(messages);
    messages
        .iter()
        .flat_map(|m| m.blocks.iter())
        .filter_map(|b| match b {
            ContentBlock::ToolUse { id, .. }
                if dangling.contains(id) && !replayable.contains(id) =>
            {
                Some(ContentBlock::ToolResult {
                    tool_use_id: id.clone(),
                    content: DANGLING_RESULT_MSG.to_string(),
                    is_error: true,
                    images: Vec::new(),
                })
            }
            _ => None,
        })
        .collect()
}

/// In-process safety net: reconcile every dangling, non-replayable `tool_use`
/// id in the live session by recording one synthetic error `ToolResult`
/// message. Idempotent — once recorded the ids are answered, so a second call
/// computes an empty dangling set. No-op when the transcript is well-formed.
///
/// Called from `run_with_registry` right after `replay_cancelled_tasks` and
/// before the new user input is recorded, so the synthesized message lands at
/// the end of the transcript (immediately before the new user turn) — the same
/// position `resume()` appends to.
pub async fn reconcile_dangling_tool_uses(session: &mut SessionState) {
    let replayable: HashSet<String> = match session.store.clone() {
        Some(store) => {
            let records = store
                .list_subagent_tasks(&session.id)
                .await
                .unwrap_or_default();
            replayable_task_ids_from_records(&records)
        }
        // Store-less session: nothing can ever be replayed, so every dangling
        // id (task included) is answered with an error result — semantically
        // more correct than leaving ids open for a replay that cannot happen.
        None => HashSet::new(),
    };
    let dangling = dangling_tool_use_results(&session.messages, &replayable);
    if dangling.is_empty() {
        return;
    }
    let n_dangling = dangling.len();
    let synthetic = Message {
        id: crate::runner::new_id(),
        role: Role::Tool,
        blocks: dangling,
        model: None,
        agent: None,
        usage: MessageUsage::default(),
        created_at: now_ms(),
        synthetic: true,
    };
    tracing::warn!(
        session_id = %session.id,
        count = n_dangling,
        "synthesizing error tool_result for dangling tool_use before run"
    );
    session.record(synthetic).await;
}
