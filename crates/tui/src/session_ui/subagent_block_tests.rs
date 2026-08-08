//! Unit tests for `build_subagent_block` status -> Subagent-block mapping.
//! Kept in a child module file so `session_ui.rs` stays within the 800-line
//! budget while retaining access to the module-private builder.

use super::replay::build_subagent_block;
use crate::chat::ChatBlock;
use opencoder_core::Message;
use opencoder_store::{SubagentStatus, SubagentTaskRecord};
use std::sync::Arc;

// ── build_subagent_block status mapping ───────────────────────────────

/// Store stub for `build_subagent_block`: an empty child transcript (no
/// events, no messages) so the reconstructed child `ChatView` is empty.
/// Everything else panics — the block builder only reads the child's
/// transcript.
struct EmptyChildStore;

#[async_trait::async_trait]
impl opencoder_store::Store for EmptyChildStore {
    fn backend_name(&self) -> &'static str {
        "empty-child-stub"
    }
    async fn load_messages(&self, _: &str) -> anyhow::Result<Vec<Message>> {
        Ok(Vec::new())
    }
    async fn events_after(
        &self,
        _: &str,
        _: i64,
    ) -> anyhow::Result<Vec<opencoder_store::SessionEventRecord>> {
        Ok(Vec::new())
    }
    async fn create_session(&self, _: &opencoder_store::SessionMeta) -> anyhow::Result<()> {
        unimplemented!()
    }
    async fn get_session(&self, _: &str) -> anyhow::Result<Option<opencoder_store::SessionMeta>> {
        unimplemented!()
    }
    async fn list_sessions(
        &self,
        _: &opencoder_store::SessionFilter,
    ) -> anyhow::Result<Vec<opencoder_store::SessionListItem>> {
        unimplemented!()
    }
    async fn update_session(
        &self,
        _: &str,
        _: &opencoder_store::SessionPatch,
    ) -> anyhow::Result<()> {
        unimplemented!()
    }
    async fn delete_session(&self, _: &str) -> anyhow::Result<()> {
        unimplemented!()
    }
    async fn clear_other_sessions(&self, _: &str) -> anyhow::Result<u64> {
        unimplemented!()
    }
    async fn append_message(&self, _: &str, _: &Message) -> anyhow::Result<i64> {
        unimplemented!()
    }
    async fn append_messages(&self, _: &str, _: &[Message]) -> anyhow::Result<Vec<i64>> {
        unimplemented!()
    }
    async fn last_message_seq(&self, _: &str) -> anyhow::Result<i64> {
        unimplemented!()
    }
    async fn admit_input(&self, _: &opencoder_store::SessionInput) -> anyhow::Result<i64> {
        unimplemented!()
    }
    async fn pending_inputs(
        &self,
        _: &str,
        _: opencoder_store::Delivery,
    ) -> anyhow::Result<Vec<opencoder_store::SessionInput>> {
        unimplemented!()
    }
    async fn promote_inputs(
        &self,
        _: &str,
        _: i64,
        _: opencoder_store::Delivery,
    ) -> anyhow::Result<Vec<i64>> {
        unimplemented!()
    }
    async fn promote_next_queued(&self, _: &str) -> anyhow::Result<Option<i64>> {
        unimplemented!()
    }
    async fn claim_next_queue(
        &self,
        _: &str,
    ) -> anyhow::Result<Option<(i64, opencoder_store::SessionInput)>> {
        unimplemented!()
    }
    async fn delete_input(&self, _: i64) -> anyhow::Result<()> {
        unimplemented!()
    }
    async fn swap_input_order(&self, _: &str, _: i64, _: i64) -> anyhow::Result<()> {
        unimplemented!()
    }
    async fn append_events(
        &self,
        _: &[opencoder_store::SessionEventRecord],
    ) -> anyhow::Result<Vec<i64>> {
        unimplemented!()
    }
    async fn last_event_seq(&self, _: &str) -> anyhow::Result<i64> {
        unimplemented!()
    }
    async fn create_subagent_task(
        &self,
        _: &opencoder_store::SubagentTaskRecord,
    ) -> anyhow::Result<()> {
        unimplemented!()
    }
    async fn complete_subagent_task(&self, _: &str, _: &str, _: bool) -> anyhow::Result<()> {
        unimplemented!()
    }
    async fn list_subagent_tasks(
        &self,
        _: &str,
    ) -> anyhow::Result<Vec<opencoder_store::SubagentTaskRecord>> {
        unimplemented!()
    }
    async fn get_subagent_task(
        &self,
        _: &str,
    ) -> anyhow::Result<Option<opencoder_store::SubagentTaskRecord>> {
        unimplemented!()
    }
    async fn cancel_subagent_task(&self, _: &str) -> anyhow::Result<()> {
        unimplemented!()
    }
}

fn task_record(
    status: SubagentStatus,
    result: Option<&str>,
    ok: Option<bool>,
) -> SubagentTaskRecord {
    SubagentTaskRecord {
        task_id: "task-1".into(),
        parent_session_id: "parent".into(),
        child_session_id: "child-1".into(),
        parent_message_id: Some("a1".into()),
        agent: "explore".into(),
        prompt: "explore the codebase".into(),
        result: result.map(str::to_string),
        status,
        ok,
        started_at: 0,
        completed_at: Some(0),
    }
}

#[tokio::test]
async fn subagent_block_completed_maps_to_done() {
    let store: Arc<dyn opencoder_store::Store> = Arc::new(EmptyChildStore);
    let task = task_record(SubagentStatus::Completed, Some("all done"), Some(true));
    let block = build_subagent_block(&task, &store).await;
    match block {
        ChatBlock::Subagent {
            done,
            ok,
            cancelled,
            summary,
            ..
        } => {
            assert!(done, "Completed task must render as done");
            assert!(ok);
            assert!(!cancelled);
            assert_eq!(summary, "all done");
        }
        other => panic!("expected Subagent block, got {other:?}"),
    }
}

#[tokio::test]
async fn subagent_block_cancelled_maps_to_cancelled_marker() {
    let store: Arc<dyn opencoder_store::Store> = Arc::new(EmptyChildStore);
    let task = task_record(SubagentStatus::Cancelled, None, None);
    let block = build_subagent_block(&task, &store).await;
    match block {
        ChatBlock::Subagent {
            done,
            ok,
            cancelled,
            summary,
            ..
        } => {
            assert!(done, "Cancelled task renders as a terminal block");
            assert!(!ok);
            assert!(cancelled, "Cancelled task must set the cancelled flag");
            assert_eq!(summary, "(cancelled)");
        }
        other => panic!("expected Subagent block, got {other:?}"),
    }
}

#[tokio::test]
async fn subagent_block_failed_maps_to_failed() {
    let store: Arc<dyn opencoder_store::Store> = Arc::new(EmptyChildStore);
    let task = task_record(SubagentStatus::Failed, Some("boom"), Some(false));
    let block = build_subagent_block(&task, &store).await;
    match block {
        ChatBlock::Subagent {
            done,
            ok,
            cancelled,
            summary,
            ..
        } => {
            assert!(done);
            assert!(!ok);
            assert!(!cancelled);
            assert_eq!(summary, "boom");
        }
        other => panic!("expected Subagent block, got {other:?}"),
    }
}

#[tokio::test]
async fn subagent_block_running_maps_to_interrupted() {
    // A Running row can only survive into the store when replay was cut
    // short (e.g. resume cancel token fired mid-replay); the rebuild must
    // still produce a terminal block rather than a dangling spinner.
    let store: Arc<dyn opencoder_store::Store> = Arc::new(EmptyChildStore);
    let task = task_record(SubagentStatus::Running, None, None);
    let block = build_subagent_block(&task, &store).await;
    match block {
        ChatBlock::Subagent {
            done,
            ok,
            cancelled,
            summary,
            ..
        } => {
            assert!(done);
            assert!(!ok);
            assert!(!cancelled);
            assert!(summary.contains("interrupted"));
        }
        other => panic!("expected Subagent block, got {other:?}"),
    }
}

#[tokio::test]
async fn replay_subagent_block_carries_duration_from_task() {
    let store: Arc<dyn opencoder_store::Store> = Arc::new(EmptyChildStore);
    let mut task = task_record(SubagentStatus::Completed, Some("done"), Some(true));
    task.started_at = 1_000_000;
    task.completed_at = Some(1_018_000);
    let block = build_subagent_block(&task, &store).await;
    match block {
        ChatBlock::Subagent {
            started_at_ms,
            elapsed_ms,
            ..
        } => {
            assert_eq!(started_at_ms, 1_000_000);
            assert_eq!(elapsed_ms, Some(18_000));
        }
        _ => panic!("expected Subagent block"),
    }
}
