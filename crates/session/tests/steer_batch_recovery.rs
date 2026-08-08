//! P1-3: steer batch error-recovery.
//!
//! When a steer batch (multiple steers claimed at one turn boundary) contains
//! an item whose processing fails, the runner must `unpromote_inputs` the
//! failed item AND all remaining unprocessed items (`steer_prompts[idx..]`)
//! so the next `claim_steers` re-absorbs them.
//!
//! These tests verify the store-level mechanism the runner relies on:
//! `unpromote_inputs` restores ALL items in the slice (not just the first)
//! and the restored items are re-claimable on the next drain. A runner-
//! integration test exercises the full batch-steer path with a failing store.

#![allow(dead_code)]

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use opencoder_core::{resolve_agent, Config, Message};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_session::{run, SessionEvent, SessionState};
use opencoder_store::{
    Delivery, ImportReport, LibsqlStore, SessionEventRecord, SessionFilter, SessionInput,
    SessionListItem, SessionMeta, SessionPatch, Store, SubagentTaskRecord,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

fn config() -> Config {
    Config {
        model: "m/g".into(),
        ..Config::default()
    }
}

fn mock_reply(text: &str) -> Arc<dyn ChatStream> {
    Arc::new(MockChatClient::new().with_default(vec![LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![],
        usage: None,
    }]))
}

fn session(store: Arc<dyn Store>, mock: Arc<dyn ChatStream>) -> (tempfile::TempDir, SessionState) {
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let s = SessionState::new(
        "recovery-sess",
        agent,
        config(),
        mock,
        dir.path().to_path_buf(),
    )
    .with_store(store);
    (dir, s)
}

/// Create the session row so input admission (FK) succeeds before the run.
async fn seed_session(store: &Arc<dyn Store>) {
    store
        .create_session(&SessionMeta {
            id: "recovery-sess".into(),
            title: Some("t".into()),
            agent: Some("act".into()),
            model: Some("m".into()),
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
        })
        .await
        .unwrap();
}

/// Admit a steer input; returns the row PK seq (admit_input's return value).
async fn admit_steer(store: &Arc<dyn Store>, session_id: &str, id: &str, prompt: &str) -> i64 {
    store
        .admit_input(&SessionInput {
            seq: None,
            id: id.into(),
            session_id: session_id.into(),
            delivery: Delivery::Steer,
            prompt: prompt.into(),
            images: Vec::new(),
            display_text: None,
            admitted_seq: 0,
            promoted_seq: None,
        })
        .await
        .unwrap()
}

/// Replicate `claim_steers` store operations: read pending steers, promote
/// them up to the max admitted_seq. Returns the promoted PK seqs in FIFO order.
async fn claim_steers_raw(store: &Arc<dyn Store>, sid: &str) -> Vec<i64> {
    let pending = store.pending_inputs(sid, Delivery::Steer).await.unwrap();
    if pending.is_empty() {
        return Vec::new();
    }
    let max_seq = pending.iter().map(|i| i.admitted_seq).max().unwrap();
    store
        .promote_inputs(sid, max_seq, Delivery::Steer)
        .await
        .unwrap()
}

/// Collect pending steer seqs (PK) in FIFO order.
async fn pending_seqs(store: &Arc<dyn Store>, sid: &str) -> Vec<i64> {
    store
        .pending_inputs(sid, Delivery::Steer)
        .await
        .unwrap()
        .iter()
        .filter_map(|i| i.seq)
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
    // Critical: delegate to inner (not the default no-op!) so the P1-3
    // recovery mechanism actually resets promoted_seq back to NULL.
    async fn unpromote_inputs(&self, sid: &str, seqs: &[i64]) -> Result<()> {
        self.inner.unpromote_inputs(sid, seqs).await
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// P1-3 core: when a steer batch fails at the first item, `unpromote_inputs`
/// must restore the failed item AND all remaining items to pending so the
/// next `claim_steers` re-absorbs them.
#[tokio::test]
async fn steer_batch_failure_unpromotes_remaining_items() {
    let store = mem_store().await;
    seed_session(&store).await;

    // Admit a batch: a control command + a normal prompt.
    let seq0 = admit_steer(&store, "recovery-sess", "s0", "/plan").await;
    let seq1 = admit_steer(&store, "recovery-sess", "s1", "hello world").await;

    // --- Simulate claim_steers: read pending + promote all ---
    assert_eq!(
        pending_seqs(&store, "recovery-sess").await.len(),
        2,
        "both steers pending before claim"
    );
    let promoted = claim_steers_raw(&store, "recovery-sess").await;
    assert_eq!(promoted.len(), 2, "claim promoted both steers");
    assert_eq!(
        pending_seqs(&store, "recovery-sess").await.len(),
        0,
        "no pending steers after promote"
    );

    // --- Simulate P1-3 error recovery (item 0 failed; idx=0 → all remain) ---
    // The runner computes: remaining = steer_prompts[0..].seqs == [seq0, seq1].
    let remaining: Vec<i64> = vec![seq0, seq1];
    store
        .unpromote_inputs("recovery-sess", &remaining)
        .await
        .unwrap();

    // --- Verify ALL items restored to pending (retriable) ---
    let restored = pending_seqs(&store, "recovery-sess").await;
    assert_eq!(
        restored.len(),
        2,
        "P1-3: both the failed item and the remaining item must be restored to pending"
    );
    assert!(
        restored.contains(&seq0),
        "failed item (seq {seq0}) must be unpromoted for retry"
    );
    assert!(
        restored.contains(&seq1),
        "remaining item (seq {seq1}) must be unpromoted for retry"
    );

    // --- Verify retry: restored items are re-claimable on the next drain ---
    let re_claimed = claim_steers_raw(&store, "recovery-sess").await;
    assert_eq!(
        re_claimed.len(),
        2,
        "restored steers are re-claimable on the next drain"
    );
    assert_eq!(
        pending_seqs(&store, "recovery-sess").await.len(),
        0,
        "no stranding after re-claim"
    );
}

/// P1-3 partial batch: when failure occurs at `idx > 0`, only items from
/// `idx` onward are unpromoted — items before `idx` were consumed and must
/// NOT be restored.
#[tokio::test]
async fn partial_batch_failure_unpromotes_only_remaining() {
    let store = mem_store().await;
    seed_session(&store).await;

    let seq0 = admit_steer(&store, "recovery-sess", "p0", "first prompt").await;
    let seq1 = admit_steer(&store, "recovery-sess", "p1", "/plan").await;
    let seq2 = admit_steer(&store, "recovery-sess", "p2", "third prompt").await;

    // Claim all three.
    let promoted = claim_steers_raw(&store, "recovery-sess").await;
    assert_eq!(promoted.len(), 3, "claim promoted all three steers");

    // Item 0 is processed successfully (stays promoted). Item 1 fails.
    // The runner unpromotes steer_prompts[1..] == [seq1, seq2].
    let remaining: Vec<i64> = vec![seq1, seq2];
    store
        .unpromote_inputs("recovery-sess", &remaining)
        .await
        .unwrap();

    // Item 0 must NOT be in the pending set (it was consumed).
    let restored = pending_seqs(&store, "recovery-sess").await;
    assert_eq!(
        restored.len(),
        2,
        "only the failed item and items after it are restored"
    );
    assert!(
        !restored.contains(&seq0),
        "consumed item (seq {seq0}) must NOT be unpromoted"
    );
    assert!(
        restored.contains(&seq1),
        "failed item (seq {seq1}) must be restored for retry"
    );
    assert!(
        restored.contains(&seq2),
        "remaining item (seq {seq2}) must be restored for retry"
    );
}

/// Runner integration (P1-2 + P1-3): with a `FailingUpdateStore`
/// (`update_session` rejects every write), `/plan` triggers
/// `persist_agent` which now PROPAGATES the error (P1-2 fix changed
/// `let _ =` to `?`). This activates P1-3 recovery: the failed steer AND
/// all remaining unprocessed steers are unpromoted for retry.
#[tokio::test]
async fn runner_consumes_batch_steers_with_failing_store() {
    let inner = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let failing: Arc<dyn Store> = Arc::new(FailingUpdateStore { inner });

    let mock = mock_reply("ok");
    let (_dir, mut s) = session(failing.clone(), mock);
    seed_session(&failing).await;

    // Admit a batch: control command + normal prompt.
    admit_steer(&failing, "recovery-sess", "r0", "/plan").await;
    admit_steer(&failing, "recovery-sess", "r1", "hello steer").await;

    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev_clone = events.clone();
    let outcome = run(&mut s, "kickoff".into(), move |ev| {
        ev_clone.lock().unwrap().push(ev)
    })
    .await;

    // P1-2: persist_agent error now propagates (was swallowed by `let _ =`).
    assert!(
        outcome.is_err(),
        "run must fail - persist_agent error propagated (P1-2)"
    );

    // The first steer (r0) was consumed (SteerConsumed emitted) before
    // the error occurred; the second was never reached.
    let consumed = events
        .lock()
        .unwrap()
        .iter()
        .filter(|ev| matches!(ev, SessionEvent::SteerConsumed { .. }))
        .count();
    assert_eq!(
        consumed, 1,
        "first steer consumed before error; second not reached"
    );

    // P1-3: both steers are unpromoted (still pending for retry).
    let pending = failing
        .pending_inputs("recovery-sess", Delivery::Steer)
        .await
        .unwrap();
    assert_eq!(
        pending.len(),
        2,
        "P1-3: both steers unpromoted for retry"
    );
}
