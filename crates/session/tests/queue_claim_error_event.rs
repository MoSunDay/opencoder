//! Regression test for **P2-4 (drain claim failure surfacing)**: when the
//! queue-drain `claim_next_queue` store call fails persistently (both the
//! initial attempt and the single retry), the runner used to only `warn!` to
//! the log and treat it as an empty queue — the pending row was stranded with
//! NO event-stream signal (silent from the UI's point of view) while run_loop
//! reported a normal `Done`.
//!
//! Fix under test: the persistent-failure path emits a
//! `SessionEvent::Error("queued input claim failed: ...")` so display surfaces
//! can see the stranded input, while the run still terminates normally
//! (Empty semantics → Done) instead of erroring the whole run.
//!
//! The failing store is a thin delegating wrapper around a real in-memory
//! libsql store whose `claim_next_queue` ALWAYS returns Err; every other
//! method (notably `pending_inputs`, so `has_pending_queues` sees the row and
//! the drain path actually engages) delegates unchanged.

use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use opencoder_core::{resolve_agent, Config, Message};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient, Usage};
use opencoder_session::{run, SessionEvent, SessionState};
use opencoder_store::{
    Delivery, LibsqlStore, SessionEventRecord, SessionFilter, SessionInput, SessionListItem,
    SessionMeta, SessionPatch, Store, SubagentTaskRecord,
};

/// Delegating store wrapper whose queue claim always fails.
struct FailingClaimStore {
    inner: Arc<dyn Store>,
}

#[async_trait]
impl Store for FailingClaimStore {
    fn backend_name(&self) -> &'static str {
        self.inner.backend_name()
    }
    async fn create_session(&self, meta: &SessionMeta) -> Result<()> {
        self.inner.create_session(meta).await
    }
    async fn get_session(&self, id: &str) -> Result<Option<SessionMeta>> {
        self.inner.get_session(id).await
    }
    async fn list_sessions(&self, f: &SessionFilter) -> Result<Vec<SessionListItem>> {
        self.inner.list_sessions(f).await
    }
    async fn update_session(&self, id: &str, patch: &SessionPatch) -> Result<()> {
        self.inner.update_session(id, patch).await
    }
    async fn delete_session(&self, id: &str) -> Result<()> {
        self.inner.delete_session(id).await
    }
    async fn clear_other_sessions(&self, keep: &str) -> Result<u64> {
        self.inner.clear_other_sessions(keep).await
    }
    async fn append_message(&self, sid: &str, msg: &Message) -> Result<i64> {
        self.inner.append_message(sid, msg).await
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
    // Deliberately delegates: the pending peek must still see the stranded
    // row so the drain path engages and the failure is actually exercised.
    async fn pending_inputs(&self, sid: &str, delivery: Delivery) -> Result<Vec<SessionInput>> {
        self.inner.pending_inputs(sid, delivery).await
    }
    async fn promote_inputs(
        &self,
        sid: &str,
        up_to_admitted_seq: i64,
        delivery: Delivery,
    ) -> Result<Vec<i64>> {
        self.inner
            .promote_inputs(sid, up_to_admitted_seq, delivery)
            .await
    }
    async fn promote_next_queued(&self, sid: &str) -> Result<Option<i64>> {
        self.inner.promote_next_queued(sid).await
    }
    /// Always fails: models a persistent store-level claim failure (lock
    /// corruption, I/O error, ...). The drain retries once and then must
    /// surface an Error event instead of failing silently.
    async fn claim_next_queue(&self, _sid: &str) -> Result<Option<(i64, SessionInput)>> {
        Err(anyhow!("simulated persistent claim failure"))
    }
    async fn delete_input(&self, input_id: i64) -> Result<()> {
        self.inner.delete_input(input_id).await
    }
    async fn swap_input_order(&self, sid: &str, a: i64, b: i64) -> Result<()> {
        self.inner.swap_input_order(sid, a, b).await
    }
    async fn append_events(&self, events: &[SessionEventRecord]) -> Result<Vec<i64>> {
        self.inner.append_events(events).await
    }
    async fn events_after(&self, sid: &str, after_seq: i64) -> Result<Vec<SessionEventRecord>> {
        self.inner.events_after(sid, after_seq).await
    }
    async fn last_event_seq(&self, sid: &str) -> Result<i64> {
        self.inner.last_event_seq(sid).await
    }
    async fn create_subagent_task(&self, rec: &SubagentTaskRecord) -> Result<()> {
        self.inner.create_subagent_task(rec).await
    }
    async fn complete_subagent_task(&self, task_id: &str, result: &str, ok: bool) -> Result<()> {
        self.inner.complete_subagent_task(task_id, result, ok).await
    }
    async fn list_subagent_tasks(
        &self,
        parent_session_id: &str,
    ) -> Result<Vec<SubagentTaskRecord>> {
        self.inner.list_subagent_tasks(parent_session_id).await
    }
    async fn get_subagent_task(&self, task_id: &str) -> Result<Option<SubagentTaskRecord>> {
        self.inner.get_subagent_task(task_id).await
    }
    async fn cancel_subagent_task(&self, task_id: &str) -> Result<()> {
        self.inner.cancel_subagent_task(task_id).await
    }
}

fn config() -> Config {
    Config {
        model: "m/g".into(),
        ..Config::default()
    }
}

fn stream_turn(text: &str) -> Vec<LlmEvent> {
    vec![
        LlmEvent::TextDelta(text.into()),
        LlmEvent::Completed {
            text: text.into(),
            tool_calls: vec![],
            usage: Some(Usage::default()),
        },
    ]
}

async fn seed(store: &Arc<dyn Store>, id: &str, agent: &str) {
    store
        .create_session(&SessionMeta {
            id: id.into(),
            agent: Some(agent.into()),
            model: Some("m/g".into()),
            created_at: 0,
            updated_at: 0,
            ..Default::default()
        })
        .await
        .unwrap();
}

fn mk_input(session_id: &str, delivery: Delivery, prompt: &str) -> SessionInput {
    SessionInput {
        seq: None,
        id: opencoder_session::runner::new_id(),
        session_id: session_id.into(),
        delivery,
        prompt: prompt.into(),
        images: vec![],
        display_text: None,
        admitted_seq: 0,
        promoted_seq: None,
    }
}

/// A persistently failing queue claim must surface as a SessionEvent::Error
/// carrying the failure context, while the run itself still terminates Ok
/// with a Done event (Empty semantics preserved) and never consumes the
/// stranded row (exactly one LLM call for the kickoff turn).
#[tokio::test]
async fn persistent_claim_failure_emits_error_event_and_run_terminates() {
    let inner: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let store: Arc<dyn Store> = Arc::new(FailingClaimStore {
        inner: inner.clone(),
    });
    seed(&inner, "claim-fail-sess", "act").await;

    // A queued follow-up that the (failing) claim can never pop.
    inner
        .admit_input(&mk_input(
            "claim-fail-sess",
            Delivery::Queue,
            "queued follow-up",
        ))
        .await
        .unwrap();

    let mock = Arc::new(MockChatClient::new().push_script(stream_turn("kickoff reply")));

    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "claim-fail-sess",
        resolve_agent("act").unwrap(),
        config(),
        mock.clone() as Arc<dyn ChatStream>,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();

    let events = Arc::new(Mutex::new(Vec::new()));
    let ev_clone = events.clone();
    let run_result = run(&mut session, "kickoff".into(), move |ev| {
        ev_clone.lock().unwrap().push(ev);
    })
    .await;

    // The run must still terminate normally (Empty semantics → Done), not
    // propagate the store error as a run failure.
    run_result.expect("run must terminate Ok despite persistent claim failure");

    let events = events.lock().unwrap();
    // The failure must be visible on the event stream — pre-fix there was no
    // Error at all, leaving the stranded row silent.
    let claim_errors: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            SessionEvent::Error(msg) if msg.contains("queued input claim failed") => Some(msg),
            _ => None,
        })
        .collect();
    assert!(
        !claim_errors.is_empty(),
        "persistent claim failure must emit SessionEvent::Error with context, \
         got events: {:?}",
        events.iter().map(|e| e.sse_kind()).collect::<Vec<_>>()
    );
    assert!(
        claim_errors
            .iter()
            .any(|m| m.contains("simulated persistent claim failure")),
        "the Error payload must carry the underlying failure: {claim_errors:?}"
    );
    // Run still ends cleanly.
    assert!(
        events.iter().any(|e| matches!(e, SessionEvent::Done)),
        "run must still emit Done after the surfaced claim failure"
    );
    // The stranded row was never consumed: exactly the kickoff LLM turn ran.
    assert_eq!(
        mock.call_count(),
        1,
        "the queue row must not be consumed into an LLM turn"
    );
}
