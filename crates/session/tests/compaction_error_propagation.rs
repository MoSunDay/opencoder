//! Compaction error-propagation: when the durable store rejects the
//! `update_session` write that persists compaction metadata, `compact()` must
//! propagate the error (`Err`) rather than silently swallowing it (the
//! pre-hardening `let _ =` behaviour). This is the error-path companion to the
//! happy-path integration coverage in `compaction_and_model.rs`.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use opencoder_core::{resolve_agent, Config, Message};
use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::{compaction::compact, SessionEvent, SessionState};
use opencoder_store::{
    Delivery, ImportReport, LibsqlStore, SessionEventRecord, SessionFilter, SessionInput,
    SessionListItem, SessionMeta, SessionPatch, Store, SubagentTaskRecord,
};

/// Wraps a real in-memory `LibsqlStore` but makes `update_session` fail,
/// simulating a transient disk/DB error during compaction metadata persistence.
struct FailingUpdateStore {
    inner: Arc<LibsqlStore>,
}

#[async_trait]
impl Store for FailingUpdateStore {
    fn backend_name(&self) -> &'static str {
        self.inner.backend_name()
    }
    async fn create_session(&self, m: &SessionMeta) -> Result<()> {
        self.inner.create_session(m).await
    }
    async fn get_session(&self, id: &str) -> Result<Option<SessionMeta>> {
        self.inner.get_session(id).await
    }
    async fn list_sessions(&self, f: &SessionFilter) -> Result<Vec<SessionListItem>> {
        self.inner.list_sessions(f).await
    }
    async fn update_session(&self, _id: &str, _patch: &SessionPatch) -> Result<()> {
        Err(anyhow::anyhow!("simulated store failure on update_session"))
    }
    async fn delete_session(&self, id: &str) -> Result<()> {
        self.inner.delete_session(id).await
    }
    async fn clear_other_sessions(&self, keep: &str) -> Result<u64> {
        self.inner.clear_other_sessions(keep).await
    }
    async fn append_message(&self, sid: &str, m: &Message) -> Result<i64> {
        self.inner.append_message(sid, m).await
    }
    async fn append_messages(&self, sid: &str, msgs: &[Message]) -> Result<Vec<i64>> {
        self.inner.append_messages(sid, msgs).await
    }
    async fn load_messages(&self, sid: &str) -> Result<Vec<Message>> {
        self.inner.load_messages(sid).await
    }
    async fn last_message_seq(&self, sid: &str) -> Result<i64> {
        self.inner.last_message_seq(sid).await
    }
    async fn admit_input(&self, input: &SessionInput) -> Result<i64> {
        self.inner.admit_input(input).await
    }
    async fn pending_inputs(&self, sid: &str, d: Delivery) -> Result<Vec<SessionInput>> {
        self.inner.pending_inputs(sid, d).await
    }
    async fn promote_inputs(&self, sid: &str, up_to: i64, d: Delivery) -> Result<Vec<i64>> {
        self.inner.promote_inputs(sid, up_to, d).await
    }
    async fn promote_next_queued(&self, sid: &str) -> Result<Option<i64>> {
        self.inner.promote_next_queued(sid).await
    }
    async fn claim_next_queue(&self, sid: &str) -> Result<Option<(i64, SessionInput)>> {
        self.inner.claim_next_queue(sid).await
    }
    async fn delete_input(&self, id: i64) -> Result<()> {
        self.inner.delete_input(id).await
    }
    async fn swap_input_order(&self, sid: &str, a: i64, b: i64) -> Result<()> {
        self.inner.swap_input_order(sid, a, b).await
    }
    async fn append_events(&self, evs: &[SessionEventRecord]) -> Result<Vec<i64>> {
        self.inner.append_events(evs).await
    }
    async fn events_after(&self, sid: &str, after: i64) -> Result<Vec<SessionEventRecord>> {
        self.inner.events_after(sid, after).await
    }
    async fn last_event_seq(&self, sid: &str) -> Result<i64> {
        self.inner.last_event_seq(sid).await
    }
    async fn create_subagent_task(&self, r: &SubagentTaskRecord) -> Result<()> {
        self.inner.create_subagent_task(r).await
    }
    async fn complete_subagent_task(&self, id: &str, res: &str, ok: bool) -> Result<()> {
        self.inner.complete_subagent_task(id, res, ok).await
    }
    async fn list_subagent_tasks(&self, sid: &str) -> Result<Vec<SubagentTaskRecord>> {
        self.inner.list_subagent_tasks(sid).await
    }
    async fn get_subagent_task(&self, id: &str) -> Result<Option<SubagentTaskRecord>> {
        self.inner.get_subagent_task(id).await
    }
    async fn cancel_subagent_task(&self, id: &str) -> Result<()> {
        self.inner.cancel_subagent_task(id).await
    }
    async fn import_messages(&self, sid: &str, msgs: &[Message]) -> Result<ImportReport> {
        self.inner.import_messages(sid, msgs).await
    }
}

/// When the store rejects the `update_session` write that persists compaction
/// metadata, `compact()` must return `Err` carrying the "persist compaction
/// metadata" context tag — not silently drop the failure. The summary LLM call
/// succeeds (MockChatClient) so we reach the store-write line.
#[tokio::test]
async fn compact_returns_err_when_store_rejects_metadata_persistence() {
    let inner = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let failing: Arc<dyn Store> = Arc::new(FailingUpdateStore { inner });

    let mock: Arc<dyn ChatStream> =
        Arc::new(
            MockChatClient::new().with_default(vec![LlmEvent::Completed {
                text: "conversation summary".into(),
                tool_calls: Vec::<CompletedToolCall>::new(),
                usage: Some(Usage {
                    input_tokens: 5,
                    output_tokens: 3,
                    total_tokens: 8,
                    ..Default::default()
                }),
            }]),
        );

    let agent = resolve_agent("act").expect("act agent resolves");
    let mut s = SessionState::new(
        "compact-err",
        agent,
        Config {
            model: "main/glm-5.2".into(),
            ..Config::default()
        },
        mock,
        std::env::temp_dir(),
    )
    .with_store(failing);

    // Two turns so compaction_split returns a real head/tail (head summarizable).
    s.messages.push(Message::user("u1", "first turn"));
    s.messages.push(Message::assistant("a1"));
    s.messages.push(Message::user("u2", "second turn"));
    s.messages.push(Message::assistant("a2"));

    let mut events: Vec<SessionEvent> = Vec::new();
    let outcome = compact(&mut s, &HashMap::new(), &mut |ev| events.push(ev)).await;

    assert!(
        outcome.is_err(),
        "compact must return Err when the store rejects the metadata write"
    );
    let msg = format!("{:#}", outcome.as_ref().unwrap_err());
    assert!(
        msg.contains("persist compaction metadata"),
        "error must carry the 'persist compaction metadata' context tag, got: {msg}"
    );
    // The "compacting" status is emitted before the failing store write.
    assert!(
        events
            .iter()
            .any(|ev| matches!(ev, SessionEvent::Status(m) if m == "compacting conversation…")),
        "compaction must report a compacting status before the store write"
    );
}
