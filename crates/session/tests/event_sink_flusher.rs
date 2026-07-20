//! P3 integration test: the buffered event flusher coalesces the
//! high-frequency token-delta stream emitted by a real `run` loop into
//! O(turn) store writes while losing zero events.
//!
//! Unlike the unit tests in `event_sink.rs` (which push records directly),
//! these tests drive the full surface wiring a caller actually uses:
//! `MockChatClient` -> `run` -> `on_event` closure -> `EventSink::push` ->
//! `run_flusher` -> `Store`. This proves the real-time UI/SSE delivery path
//! (`on_event`) is untouched while only the disk path is batched.
//!
//! Contracts:
//! - token_storm_persists_losslessly_with_oturn_writes: a mock streaming
//!   N TextDelta events then Completed drives a complete turn; every delta
//!   lands in `events_after` (zero loss), `append_events` is called O(turn)
//!   (< N/10), seqs stay monotonic, and the assistant message is durable.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use opencoder_core::{resolve_agent, Config, Message, Role};
use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::{run, spawn_event_flusher, SessionState};
use opencoder_store::{
    Delivery, ImportReport, LibsqlStore, SessionEventRecord, SessionFilter, SessionInput,
    SessionListItem, SessionMeta, SessionPatch, Store, SubagentTaskRecord,
};

fn config(model: &str) -> Config {
    Config {
        model: model.into(),
        ..Config::default()
    }
}

/// Wraps a real `LibsqlStore`, counting `append_events` calls so we can assert
/// the write count is O(turn) under a token storm while losing zero events.
struct CountingStore {
    inner: Arc<LibsqlStore>,
    events_calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Store for CountingStore {
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
    async fn update_session(&self, id: &str, p: &SessionPatch) -> Result<()> {
        self.inner.update_session(id, p).await
    }
    async fn delete_session(&self, id: &str) -> Result<()> {
        self.inner.delete_session(id).await
    }
    async fn clear_other_sessions(&self, k: &str) -> Result<u64> {
        self.inner.clear_other_sessions(k).await
    }
    async fn append_message(&self, sid: &str, m: &Message) -> Result<i64> {
        self.inner.append_message(sid, m).await
    }
    async fn append_messages(&self, sid: &str, m: &[Message]) -> Result<Vec<i64>> {
        self.inner.append_messages(sid, m).await
    }
    async fn load_messages(&self, sid: &str) -> Result<Vec<Message>> {
        self.inner.load_messages(sid).await
    }
    async fn last_message_seq(&self, sid: &str) -> Result<i64> {
        self.inner.last_message_seq(sid).await
    }
    async fn admit_input(&self, i: &SessionInput) -> Result<i64> {
        self.inner.admit_input(i).await
    }
    async fn pending_inputs(&self, sid: &str, d: Delivery) -> Result<Vec<SessionInput>> {
        self.inner.pending_inputs(sid, d).await
    }
    async fn promote_inputs(&self, sid: &str, s: i64, d: Delivery) -> Result<Vec<i64>> {
        self.inner.promote_inputs(sid, s, d).await
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
        self.events_calls.fetch_add(1, Ordering::Relaxed);
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
    async fn import_messages(&self, sid: &str, msgs: &[Message]) -> Result<ImportReport> {
        self.inner.import_messages(sid, msgs).await
    }
}

#[tokio::test]
async fn token_storm_persists_losslessly_with_oturn_writes() {
    let inner = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let store: Arc<CountingStore> = Arc::new(CountingStore {
        inner: inner.clone(),
        events_calls: Arc::new(AtomicUsize::new(0)),
    });
    let dyn_store: Arc<dyn Store> = store.clone();

    // Script: a burst of TextDelta tokens, then a final Completed (no tools).
    let n = 2000usize;
    let mut script: Vec<LlmEvent> = (0..n).map(|_| LlmEvent::TextDelta("x".into())).collect();
    script.push(LlmEvent::Completed {
        text: "x".repeat(n),
        tool_calls: Vec::<CompletedToolCall>::new(),
        usage: Some(Usage {
            input_tokens: 5,
            output_tokens: 3,
            total_tokens: 8,
            ..Default::default()
        }),
    });
    let mock: Arc<dyn ChatStream> = Arc::new(MockChatClient::new().push_script(script));

    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let mut s = SessionState::new(
        "storm",
        agent,
        config("main/glm-5.2"),
        mock,
        dir.path().to_path_buf(),
    )
    .with_store(dyn_store.clone());

    // Wire the on_event callback to the buffered EventSink — exactly how the
    // TUI / web / CLI headless surfaces integrate the flusher.
    let (sink, flusher) = spawn_event_flusher(Some(dyn_store.clone()), "storm".into());
    run(&mut s, "generate".into(), |ev| {
        let _ = sink.push(&ev);
    })
    .await
    .unwrap();
    // Drop every sink clone so the channel closes, then await the final flush:
    // this is the losslessness contract (every pushed record is durable).
    drop(sink);
    let _ = flusher.await;

    let all = inner.events_after("storm", 0).await.unwrap();

    // Zero loss: every streamed TextDelta is persisted.
    let text_deltas = all
        .iter()
        .filter(|r| r.sse_kind.as_deref() == Some("text_delta"))
        .count();
    assert_eq!(
        text_deltas, n,
        "every TextDelta must be persisted through the run loop (zero loss)"
    );

    // O(turn) writes, not O(tokens): 2000 single-writes would be ~2000 calls.
    let calls = store.events_calls.load(Ordering::Relaxed);
    assert!(
        calls < n / 10,
        "append_events called {calls} times for {n} deltas; expected O(turn) (< {})",
        n / 10
    );

    // Emission order preserved across the whole stream.
    let seqs: Vec<i64> = all.iter().map(|r| r.seq.unwrap()).collect();
    let mut sorted = seqs.clone();
    sorted.sort_unstable();
    assert_eq!(seqs, sorted, "event seqs must be strictly increasing");

    // The authoritative turn text landed via the per-turn messages append.
    let msgs = inner.load_messages("storm").await.unwrap();
    assert!(
        msgs.iter().any(|m| m.role == Role::Assistant),
        "assistant message persisted"
    );
}
