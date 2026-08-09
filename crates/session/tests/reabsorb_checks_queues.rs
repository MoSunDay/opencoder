//! Integration test for P2#9: the re-absorb tail in `run_with_registry`
//! (`crates/session/src/runner/mod.rs`) must check `has_pending_queues` in
//! addition to `has_pending_steers`.
//!
//! Without that fix, a queue input admitted *after* `run_loop`'s final in-loop
//! poll (the late peek that normally catches queues) would be stranded until
//! the next manual submit.
//!
//! This test is **deterministic** — it would FAIL against a tree where the
//! re-absorb tail only consulted `has_pending_steers`. To make the timing
//! deterministic we wrap the real `LibsqlStore` in a `GatedStore` that hides
//! queue-delivery inputs from `pending_inputs` / `claim_next_queue` until an
//! `AtomicBool` gate is flipped open inside the `Done` event callback. That
//! ordering guarantees the in-loop late-peek sees nothing (gate still closed)
//! while the subsequent re-absorb tail sees the queue (gate now open).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use async_trait::async_trait;
use opencoder_core::{resolve_agent, Config, Message};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient, Usage};
use opencoder_session::{run, SessionEvent, SessionState};
use opencoder_store::{
    Delivery, LibsqlStore, SessionEventRecord, SessionFilter, SessionInput, SessionListItem,
    SessionMeta, SessionPatch, Store, SubagentTaskRecord,
};

/// Store wrapper that conceals queue inputs until `reveal` is set true.
///
/// Every method delegates to `inner` unchanged EXCEPT `pending_inputs` and
/// `claim_next_queue`, which pretend there are no queued inputs while the
/// gate is closed. This forces a queue that *is* persisted to survive the
/// in-loop late peek and only become visible to the re-absorb tail (after the
/// `Done` callback opens the gate).
struct GatedStore {
    inner: Arc<dyn Store>,
    reveal: Arc<AtomicBool>,
}

#[async_trait]
impl Store for GatedStore {
    fn backend_name(&self) -> &'static str {
        self.inner.backend_name()
    }

    async fn create_session(&self, meta: &SessionMeta) -> Result<()> {
        self.inner.create_session(meta).await
    }
    async fn get_session(&self, id: &str) -> Result<Option<SessionMeta>> {
        self.inner.get_session(id).await
    }
    async fn list_sessions(&self, filter: &SessionFilter) -> Result<Vec<SessionListItem>> {
        self.inner.list_sessions(filter).await
    }
    async fn update_session(&self, id: &str, patch: &SessionPatch) -> Result<()> {
        self.inner.update_session(id, patch).await
    }
    async fn delete_session(&self, id: &str) -> Result<()> {
        self.inner.delete_session(id).await
    }
    async fn clear_other_sessions(&self, keep_session_id: &str) -> Result<u64> {
        self.inner.clear_other_sessions(keep_session_id).await
    }

    async fn append_message(&self, session_id: &str, msg: &Message) -> Result<i64> {
        self.inner.append_message(session_id, msg).await
    }
    async fn append_messages(&self, session_id: &str, msgs: &[Message]) -> Result<Vec<i64>> {
        self.inner.append_messages(session_id, msgs).await
    }
    async fn load_messages(&self, session_id: &str) -> Result<Vec<Message>> {
        self.inner.load_messages(session_id).await
    }
    async fn last_message_seq(&self, session_id: &str) -> Result<i64> {
        self.inner.last_message_seq(session_id).await
    }

    async fn admit_input(&self, input: &SessionInput) -> Result<i64> {
        self.inner.admit_input(input).await
    }

    /// Hide queued inputs until the gate opens.
    async fn pending_inputs(
        &self,
        session_id: &str,
        delivery: Delivery,
    ) -> Result<Vec<SessionInput>> {
        let inputs = self.inner.pending_inputs(session_id, delivery).await?;
        if delivery == Delivery::Queue && !self.reveal.load(Ordering::SeqCst) {
            Ok(vec![])
        } else {
            Ok(inputs)
        }
    }

    async fn promote_inputs(
        &self,
        session_id: &str,
        up_to_admitted_seq: i64,
        delivery: Delivery,
    ) -> Result<Vec<i64>> {
        self.inner
            .promote_inputs(session_id, up_to_admitted_seq, delivery)
            .await
    }
    async fn promote_next_queued(&self, session_id: &str) -> Result<Option<i64>> {
        self.inner.promote_next_queued(session_id).await
    }

    /// Refuse to claim a queue until the gate opens.
    async fn claim_next_queue(&self, session_id: &str) -> Result<Option<(i64, SessionInput)>> {
        if !self.reveal.load(Ordering::SeqCst) {
            Ok(None)
        } else {
            self.inner.claim_next_queue(session_id).await
        }
    }

    async fn delete_input(&self, input_id: i64) -> Result<()> {
        self.inner.delete_input(input_id).await
    }
    async fn swap_input_order(&self, session_id: &str, seq_a: i64, seq_b: i64) -> Result<()> {
        self.inner.swap_input_order(session_id, seq_a, seq_b).await
    }

    async fn append_events(&self, events: &[SessionEventRecord]) -> Result<Vec<i64>> {
        self.inner.append_events(events).await
    }
    async fn events_after(
        &self,
        session_id: &str,
        after_seq: i64,
    ) -> Result<Vec<SessionEventRecord>> {
        self.inner.events_after(session_id, after_seq).await
    }
    async fn last_event_seq(&self, session_id: &str) -> Result<i64> {
        self.inner.last_event_seq(session_id).await
    }

    async fn create_subagent_task(&self, record: &SubagentTaskRecord) -> Result<()> {
        self.inner.create_subagent_task(record).await
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

// ---- helpers (mirrors of crates/session/tests/queue_echo.rs) ----------------

fn config() -> Config {
    Config {
        model: "m/g".into(),
        ..Config::default()
    }
}

/// A scripted turn: one text delta then completion with empty tool calls.
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

/// Create a session row directly in the *inner* (ungated) store so the gate
/// never interferes with the session's own creation.
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

/// A queue admitted after `run_loop`'s final in-loop late peek must still be
/// drained by the re-absorb tail in `run_with_registry`, which (post-P2#9)
/// consults `has_pending_queues` alongside `has_pending_steers`.
#[tokio::test]
async fn reabsorb_tail_picks_up_queued_input_missed_by_in_loop_poll() {
    let inner: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let reveal = Arc::new(AtomicBool::new(false));
    let store: Arc<dyn Store> = Arc::new(GatedStore {
        inner: inner.clone(),
        reveal: reveal.clone(),
    });
    seed(&inner, "reabsorb-sess", "act").await;

    // Admit a queue input — it's persisted but HIDDEN by the closed gate, so
    // the in-loop `claim_one_queued` / late-peek both miss it.
    inner
        .admit_input(&mk_input(
            "reabsorb-sess",
            Delivery::Queue,
            "follow-up task",
        ))
        .await
        .unwrap();

    let mock = Arc::new(
        MockChatClient::new()
            .push_script(stream_turn("kickoff reply"))
            .push_script(stream_turn("queue reply")),
    );

    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "reabsorb-sess",
        resolve_agent("act").unwrap(),
        config(),
        mock.clone() as Arc<dyn ChatStream>,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();

    let events = Arc::new(Mutex::new(Vec::new()));
    let ev_clone = events.clone();
    let reveal_cb = reveal.clone();
    run(&mut session, "kickoff".into(), move |ev| {
        // Open the gate when Done fires — strictly AFTER the in-loop late
        // peek (which runs before Done is emitted) but BEFORE the re-absorb
        // tail consults `has_pending_queues`.
        if matches!(ev, SessionEvent::Done) {
            reveal_cb.store(true, Ordering::SeqCst);
        }
        ev_clone.lock().unwrap().push(ev);
    })
    .await
    .unwrap();

    // WITH the fix: the re-absorb tail sees the now-revealed queue and
    // re-enters run_loop, consuming it and producing a second LLM turn.
    // WITHOUT the fix (only `has_pending_steers`): the tail exits, the queue
    // is stranded, and `call_count` stays at 1.
    assert_eq!(
        mock.call_count(),
        2,
        "re-absorb tail should pick up the queue for a second LLM turn \
         (got {} calls)",
        mock.call_count()
    );

    // The follow-up must actually be consumed, not merely visible.
    let evs = events.lock().unwrap();
    assert!(
        evs.iter().any(|e| matches!(
            e,
            SessionEvent::QueueConsumed { text, .. } if text == "follow-up task"
        )),
        "the queued follow-up must be consumed"
    );
}
