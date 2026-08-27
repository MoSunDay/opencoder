//! F2 + F3 integration: durable input delivery and crash/cancel recovery.
//!
//! F2 — the `recorded` marker: every consumed input (steer or queue) is
//! marked recorded immediately after its consumption, and a run's entry
//! flips promoted-but-unrecorded orphans (crash / hard-cancel between
//! promote and consume) back to pending so this run re-absorbs them.
//!
//! F3 — zero-resubmit on failure: a failed run (LLM error or store error)
//! never auto-resubmits admitted inputs. The invariant is no-strand — an
//! input is either consumed, or pending, never stranded-promoted (invisible
//! to pending polls and never recorded) — and a failed attempt fires no
//! additional LLM requests for pending rows; the next successful run
//! consumes them.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use opencoder_core::{resolve_agent, Config, ContentBlock, Message, Role};
use opencoder_llm::{ChatRequest, ChatStream, LlmEvent, MockChatClient};
use opencoder_session::{run, SessionState};
use opencoder_store::{
    Delivery, LibsqlStore, SessionEventRecord, SessionFilter, SessionInput, SessionListItem,
    SessionMeta, SessionPatch, Store, SubagentTaskRecord,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use common::{mem_store, session_meta};

const SID: &str = "f2-recovery-sess";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn config() -> Config {
    Config {
        model: "m/g".into(),
        ..Config::default()
    }
}

fn mock_reply() -> Arc<dyn ChatStream> {
    Arc::new(
        MockChatClient::new().with_default(vec![LlmEvent::Completed {
            text: "ok".into(),
            tool_calls: vec![],
            usage: None,
        }]),
    )
}

/// ChatStream whose every `chat_stream` call fails: models an LLM outage so
/// the run's first (and per the zero-resubmit contract, only) turn errors.
/// Counts calls — the number of LLM requests is the observable that pins the
/// no-auto-resubmit behavior.
#[derive(Default)]
struct AlwaysFailStream {
    calls: AtomicUsize,
}

impl ChatStream for AlwaysFailStream {
    fn chat_stream(&self, _req: ChatRequest) -> anyhow::Result<mpsc::Receiver<LlmEvent>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(anyhow::anyhow!("simulated LLM outage"))
    }
}

fn session(store: Arc<dyn Store>, mock: Arc<dyn ChatStream>) -> (tempfile::TempDir, SessionState) {
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let s = SessionState::new(
        SID.to_string(),
        agent,
        config(),
        mock,
        dir.path().to_path_buf(),
    )
    .with_store(store);
    (dir, s)
}

/// Create the session row so input admission (FK) succeeds before the run.
async fn seed(store: &Arc<dyn Store>) {
    store
        .create_session(&session_meta(SID, "act"))
        .await
        .unwrap();
}

/// Admit an input of any delivery; returns the row PK seq.
async fn admit(store: &Arc<dyn Store>, id: &str, delivery: Delivery, prompt: &str) -> i64 {
    store
        .admit_input(&SessionInput {
            seq: None,
            id: id.into(),
            session_id: SID.into(),
            delivery,
            prompt: prompt.into(),
            images: Vec::new(),
            display_text: None,
            admitted_seq: 0,
            promoted_seq: None,
        })
        .await
        .unwrap()
}

/// All text blocks of user messages, flattened (for "was it recorded?" checks).
fn user_texts(msgs: &[Message]) -> Vec<String> {
    msgs.iter()
        .filter(|m| m.role == Role::User)
        .flat_map(|m| m.blocks.iter())
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// FailingUpdateStore — delegates everything to inner except update_session
// ---------------------------------------------------------------------------

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
    // Critical: delegate to inner (not the default no-op!) so the F2/F3
    // lifecycle actually runs through the wrapper.
    async fn unpromote_inputs(&self, sid: &str, seqs: &[i64]) -> Result<()> {
        self.inner.unpromote_inputs(sid, seqs).await
    }
    async fn mark_inputs_recorded(&self, sid: &str, seqs: &[i64]) -> Result<()> {
        self.inner.mark_inputs_recorded(sid, seqs).await
    }
    async fn recover_orphan_inputs(&self, sid: &str) -> Result<u64> {
        self.inner.recover_orphan_inputs(sid).await
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
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// F2: a row promoted-but-never-recorded by a crashed prior run (invisible
/// to `pending_inputs`) is flipped back to pending at run entry and then
/// consumed by this run's drain.
#[tokio::test]
async fn orphan_recovery_reabsorbs_promoted_unrecorded_input() {
    let store = mem_store().await;
    seed(&store).await;
    admit(&store, "q0", Delivery::Queue, "orphaned follow-up").await;

    // Simulate the crash: promote directly, leaving the row promoted with
    // recorded=0 — an orphan invisible to every pending poll.
    let pending = store.pending_inputs(SID, Delivery::Queue).await.unwrap();
    let max_admitted = pending.iter().map(|i| i.admitted_seq).max().unwrap();
    store
        .promote_inputs(SID, max_admitted, Delivery::Queue)
        .await
        .unwrap();
    assert!(
        store
            .pending_inputs(SID, Delivery::Queue)
            .await
            .unwrap()
            .is_empty(),
        "orphan must be invisible to pending_inputs BEFORE the run"
    );

    let (_dir, mut s) = session(store.clone(), mock_reply());
    let out = run(&mut s, "".into(), |_| {}).await;
    assert!(out.is_ok(), "run should succeed: {out:?}");

    // Entry recovery flipped the orphan; the drain consumed and recorded it.
    let msgs = store.load_messages(SID).await.unwrap();
    assert!(
        user_texts(&msgs)
            .iter()
            .any(|t| t.contains("orphaned follow-up")),
        "orphaned input must be re-absorbed as a persisted user message"
    );
    assert!(store
        .pending_inputs(SID, Delivery::Queue)
        .await
        .unwrap()
        .is_empty());
}

/// Zero-resubmit (LLM failure): a run whose LLM turn fails must NOT
/// auto-resubmit admitted inputs. The drain claims and consumes exactly ONE
/// queued item (its turn fails), the failure aborts the run, and the second
/// item stays PENDING — no re-absorb fires another LLM request for it. The
/// stranded item is consumed by the NEXT successful run instead.
#[tokio::test]
async fn llm_failure_leaves_queue_pending_without_resubmit() {
    let store = mem_store().await;
    seed(&store).await;
    admit(&store, "q0", Delivery::Queue, "first queued").await;
    admit(&store, "q1", Delivery::Queue, "second queued").await;

    let failing = Arc::new(AlwaysFailStream::default());
    let (_dir, mut s) = session(store.clone(), failing.clone());
    let outcome = run(&mut s, "".into(), |_| {}).await;
    assert!(outcome.is_err(), "run must fail — LLM error propagated");

    // Exactly ONE LLM request was issued: the failed turn of the consumed
    // head item. No error-path re-absorb may silently re-submit the tail.
    assert_eq!(
        failing.calls.load(Ordering::SeqCst),
        1,
        "a failed run must not fire additional LLM requests"
    );

    // The consumed head item is recorded (its user message persisted); the
    // tail item survives pending — never deleted, never auto-retried.
    let msgs = store.load_messages(SID).await.unwrap();
    assert!(
        user_texts(&msgs).iter().any(|t| t.contains("first queued")),
        "consumed head item must be recorded as a user message"
    );
    let pending = store.pending_inputs(SID, Delivery::Queue).await.unwrap();
    let texts: Vec<&str> = pending.iter().map(|i| i.prompt.as_str()).collect();
    assert_eq!(
        texts,
        vec!["second queued"],
        "tail item must stay pending after the failed run: {texts:?}"
    );
    assert_eq!(
        store.recover_orphan_inputs(SID).await.unwrap(),
        0,
        "no promoted-but-unrecorded rows may survive the failed run"
    );

    // The stranded item is not lost: the next run consumes it normally.
    let (_dir, mut s) = session(store.clone(), mock_reply());
    let outcome = run(&mut s, "".into(), |_| {}).await;
    assert!(outcome.is_ok(), "recovery run should succeed: {outcome:?}");
    assert!(
        store
            .pending_inputs(SID, Delivery::Queue)
            .await
            .unwrap()
            .is_empty(),
        "the next successful run consumes the stranded tail item"
    );
}

/// Zero-resubmit (store failure): with a persistently failing
/// `update_session`, the queued "/plan" errors in `persist_agent`, which
/// propagates out of run_loop and fails the run. The in-place P1-3 guard
/// unpromotes the failed item, so the invariant is no-strand at the store
/// layer: both items remain PENDING (visible, recoverable), never
/// stranded-promoted (invisible, lost) — and the failure aborts the run
/// instead of re-entering drain mode for another consumption round.
#[tokio::test]
async fn store_failure_leaves_queue_pending_unpromoted() {
    let inner = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let failing: Arc<dyn Store> = Arc::new(FailingUpdateStore { inner });

    let (_dir, mut s) = session(failing.clone(), mock_reply());
    seed(&failing).await;
    admit(&failing, "q0", Delivery::Queue, "/plan").await;
    admit(&failing, "q1", Delivery::Queue, "plain follow-up").await;

    let outcome = run(&mut s, "".into(), |_| {}).await;
    assert!(
        outcome.is_err(),
        "run must fail — persist_agent error propagated"
    );

    // Both items: still pending (the failed item is unpromoted back in place
    // by the P1-3 guard; the tail was never claimed), nothing stranded.
    let pending = failing.pending_inputs(SID, Delivery::Queue).await.unwrap();
    let texts: Vec<&str> = pending.iter().map(|i| i.prompt.as_str()).collect();
    assert!(
        texts.contains(&"plain follow-up"),
        "second queue item must remain pending, not stranded: {texts:?}"
    );
    assert!(
        texts.contains(&"/plan"),
        "the failed item itself must be unpromoted back to pending: {texts:?}"
    );
    assert_eq!(
        failing.recover_orphan_inputs(SID).await.unwrap(),
        0,
        "no promoted-but-unrecorded rows may survive the failed run"
    );
}

/// F1 invariant, end-to-end: a hard cancel that lands BEFORE the turn
/// boundary claims nothing — the steer stays pending (promoted_seq NULL),
/// so no lost-promote orphan is created; a later healthy run consumes it.
#[tokio::test]
async fn steer_claim_survives_hard_cancel_without_lost_promote() {
    let store = mem_store().await;
    seed(&store).await;
    admit(&store, "s0", Delivery::Steer, "steer after cancel").await;

    let (_dir, mut s) = session(store.clone(), mock_reply());
    let fired = CancellationToken::new();
    fired.cancel();
    s.cancel = Some(fired);
    let out = run(&mut s, "".into(), |_| {}).await;
    assert!(out.is_ok(), "cancelled run should break cleanly: {out:?}");

    // The top-of-loop interrupt check fires BEFORE claim_steers, so the
    // steer was never promoted: pending (recoverable) and no orphan.
    let pending = store.pending_inputs(SID, Delivery::Steer).await.unwrap();
    assert_eq!(pending.len(), 1, "steer must survive the hard cancel");
    assert_eq!(
        store.recover_orphan_inputs(SID).await.unwrap(),
        0,
        "no orphan created by the cancelled run"
    );

    // A later healthy run re-absorbs it fully.
    s.cancel = Some(CancellationToken::new());
    let out = run(&mut s, "".into(), |_| {}).await;
    assert!(out.is_ok(), "recovery run should succeed: {out:?}");
    let msgs = store.load_messages(SID).await.unwrap();
    assert!(
        user_texts(&msgs)
            .iter()
            .any(|t| t.contains("steer after cancel")),
        "steer must be consumed as a persisted user message"
    );
    assert!(store
        .pending_inputs(SID, Delivery::Steer)
        .await
        .unwrap()
        .is_empty());
}
