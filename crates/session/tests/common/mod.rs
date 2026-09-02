//! Shared helpers for the session integration suites.
//!
//! Kept in a separate module so each consuming test file
//! (`resume_replay.rs`, `resume_cancelled_pending.rs`,
//! `skill_queue_drain.rs`, ...) stays within the per-file line budget
//! without duplicating the store/message fixtures.

#![allow(dead_code)] // each consuming test crate uses a different subset

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use opencoder_core::{Config, ContentBlock, Message, MessageUsage, Role};
use opencoder_llm::{CompletedToolCall, LlmEvent, Usage};
use opencoder_store::{Delivery, LibsqlStore, SessionInput, SessionMeta, SessionPatch, Store};

pub async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

pub fn config(model: &str) -> Config {
    Config {
        model: model.into(),
        ..Config::default()
    }
}

pub fn done_event(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.to_string(),
        tool_calls: Vec::<CompletedToolCall>::new(),
        usage: Some(Usage {
            input_tokens: 5,
            output_tokens: 3,
            total_tokens: 8,
            ..Default::default()
        }),
    }
}

pub fn session_meta(id: &str, agent: &str) -> SessionMeta {
    SessionMeta {
        id: id.into(),
        title: Some("test".into()),
        agent: Some(agent.into()),
        model: Some("m".into()),

        autopilot_mode: None,
        workdir_hash: None,
        created_at: 0,
        updated_at: 0,
        summary: None,
        summary_seq: None,
        summary_images: vec![],
        handoff_seq: None,
        handoff_plan: None,
        skill: None,
        task_type: None,
        requirement: None,
    }
}

/// A parent assistant turn that emits one or more `task` tool_use blocks.
pub fn parent_task_turn(task_ids: &[&str]) -> Message {
    let mut blocks: Vec<ContentBlock> = vec![ContentBlock::Text {
        text: "delegating".into(),
    }];
    for id in task_ids {
        blocks.push(ContentBlock::ToolUse {
            id: (*id).into(),
            name: "task".into(),
            input: serde_json::json!({"prompt": "explore", "subagent_type": "explore"}),
        });
    }
    Message {
        display: None,
        id: "a1".into(),
        role: Role::Assistant,
        blocks,
        model: Some("m".into()),
        agent: Some("act".into()),
        usage: MessageUsage::default(),
        created_at: 0,
        synthetic: false,
    }
}

/// Collect the set of `tool_use` ids in `msgs` that have no matching
/// `tool_result` (i.e. would trigger dangling reconciliation).
pub fn dangling_tool_uses(msgs: &[Message]) -> Vec<String> {
    let answered: HashSet<&str> = msgs
        .iter()
        .flat_map(|m| m.blocks.iter())
        .filter_map(|b| match b {
            ContentBlock::ToolResult { tool_use_id, .. } => Some(tool_use_id.as_str()),
            _ => None,
        })
        .collect();
    msgs.iter()
        .flat_map(|m| m.blocks.iter())
        .filter_map(|b| match b {
            ContentBlock::ToolUse { id, .. } if !answered.contains(id.as_str()) => Some(id.clone()),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Fault-injection store wrappers shared by integration tests
// ---------------------------------------------------------------------------

/// A store wrapper whose `claim_next_queue` fails with an Err on the FIRST
/// call only (simulating a transient BEGIN IMMEDIATE contention). The retry
/// in `claim_one_queued` must recover: the drain pops the item exactly once
/// and the run completes instead of stranding the row pending.
pub struct FlakyClaimStore {
    pub inner: Arc<dyn Store>,
    pub first_claim_failed: Mutex<bool>,
}

#[async_trait]
impl Store for FlakyClaimStore {
    fn backend_name(&self) -> &'static str {
        self.inner.backend_name()
    }
    async fn create_session(&self, meta: &SessionMeta) -> anyhow::Result<()> {
        self.inner.create_session(meta).await
    }
    async fn get_session(&self, id: &str) -> anyhow::Result<Option<SessionMeta>> {
        self.inner.get_session(id).await
    }
    async fn list_sessions(
        &self,
        filter: &opencoder_store::SessionFilter,
    ) -> anyhow::Result<Vec<opencoder_store::SessionListItem>> {
        self.inner.list_sessions(filter).await
    }
    async fn update_session(&self, id: &str, patch: &SessionPatch) -> anyhow::Result<()> {
        self.inner.update_session(id, patch).await
    }
    async fn delete_session(&self, id: &str) -> anyhow::Result<()> {
        self.inner.delete_session(id).await
    }
    async fn clear_other_sessions(&self, keep: &str) -> anyhow::Result<u64> {
        self.inner.clear_other_sessions(keep).await
    }
    async fn append_message(
        &self,
        sid: &str,
        msg: &opencoder_core::Message,
    ) -> anyhow::Result<i64> {
        self.inner.append_message(sid, msg).await
    }
    async fn append_messages(
        &self,
        sid: &str,
        msgs: &[opencoder_core::Message],
    ) -> anyhow::Result<Vec<i64>> {
        self.inner.append_messages(sid, msgs).await
    }
    async fn load_messages(&self, sid: &str) -> anyhow::Result<Vec<opencoder_core::Message>> {
        self.inner.load_messages(sid).await
    }
    async fn last_message_seq(&self, sid: &str) -> anyhow::Result<i64> {
        self.inner.last_message_seq(sid).await
    }
    async fn admit_input(&self, input: &SessionInput) -> anyhow::Result<i64> {
        self.inner.admit_input(input).await
    }
    async fn pending_inputs(
        &self,
        sid: &str,
        delivery: Delivery,
    ) -> anyhow::Result<Vec<SessionInput>> {
        self.inner.pending_inputs(sid, delivery).await
    }
    async fn promote_inputs(
        &self,
        sid: &str,
        up_to: i64,
        delivery: Delivery,
    ) -> anyhow::Result<Vec<i64>> {
        self.inner.promote_inputs(sid, up_to, delivery).await
    }
    async fn promote_next_queued(&self, sid: &str) -> anyhow::Result<Option<i64>> {
        self.inner.promote_next_queued(sid).await
    }
    async fn claim_next_queue(&self, sid: &str) -> anyhow::Result<Option<(i64, SessionInput)>> {
        if let Ok(mut guard) = self.first_claim_failed.lock() {
            if !*guard {
                *guard = true;
                return Err(anyhow::anyhow!("transient BEGIN IMMEDIATE contention"));
            }
        }
        self.inner.claim_next_queue(sid).await
    }
    async fn delete_input(&self, input_id: i64) -> anyhow::Result<()> {
        self.inner.delete_input(input_id).await
    }
    async fn swap_input_order(&self, sid: &str, a: i64, b: i64) -> anyhow::Result<()> {
        self.inner.swap_input_order(sid, a, b).await
    }
    async fn append_events(
        &self,
        events: &[opencoder_store::SessionEventRecord],
    ) -> anyhow::Result<Vec<i64>> {
        self.inner.append_events(events).await
    }
    async fn events_after(
        &self,
        sid: &str,
        after: i64,
    ) -> anyhow::Result<Vec<opencoder_store::SessionEventRecord>> {
        self.inner.events_after(sid, after).await
    }
    async fn last_event_seq(&self, sid: &str) -> anyhow::Result<i64> {
        self.inner.last_event_seq(sid).await
    }
    async fn create_subagent_task(
        &self,
        record: &opencoder_store::SubagentTaskRecord,
    ) -> anyhow::Result<()> {
        self.inner.create_subagent_task(record).await
    }
    async fn complete_subagent_task(
        &self,
        task_id: &str,
        result: &str,
        ok: bool,
    ) -> anyhow::Result<()> {
        self.inner.complete_subagent_task(task_id, result, ok).await
    }
    async fn list_subagent_tasks(
        &self,
        parent: &str,
    ) -> anyhow::Result<Vec<opencoder_store::SubagentTaskRecord>> {
        self.inner.list_subagent_tasks(parent).await
    }
    async fn get_subagent_task(
        &self,
        task_id: &str,
    ) -> anyhow::Result<Option<opencoder_store::SubagentTaskRecord>> {
        self.inner.get_subagent_task(task_id).await
    }
    async fn cancel_subagent_task(&self, task_id: &str) -> anyhow::Result<()> {
        self.inner.cancel_subagent_task(task_id).await
    }
}
