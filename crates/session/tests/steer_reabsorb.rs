#![allow(dead_code)]
//! P1-4 regression test: the bounded re-absorb loop in `run_with_registry`.
//!
//! After `run_loop()` returns, the runner re-checks for pending steers and
//! re-runs up to `MAX_RECHECKS` (3) times to absorb steers that landed in the
//! idle window — between run_loop's final `pending_inputs` poll and its return.
//! Without this safety net, a main-session steer admitted in that window would
//! be stranded until the next manual submit.
//!
//! Contract:
//! - `late_steer_reabsorbed_after_run_loop_returns`: a steer that run_loop's
//!   first invocation never observes (hidden from both its turn-boundary
//!   `claim_steers` and its idle-boundary `has_pending_steers`) is nevertheless
//!   absorbed by the outer re-check loop, producing the follow-up LLM turn and
//!   leaving no stranded pending row.
//!
//! ## How the race is made deterministic
//!
//! The real race (steer committed by an external task after run_loop's last
//! poll but before the outer re-check) is sub-microsecond and cannot be hit
//! reliably with sleeps. Instead we wrap the real `LibsqlStore` in a
//! [`DelayedSteerStore`] that *hides* pending steers from the first
//! `HIDE_UNTIL` (2) `pending_inputs(Delivery::Steer)` calls, then delegates
//! transparently. The steer row is genuinely persisted (so it can be promoted
//! later) — only its visibility is gated.
//!
//! Call trace during the first `run_loop` (drain_mode = false):
//!   1. turn-boundary `claim_steers`  → pending_inputs #0 (hidden → empty)
//!   2. idle-boundary `has_pending_steers` → pending_inputs #1 (hidden → Done)
//!
//! Then P1-4's outer loop:
//!
//!   3. `has_pending_steers` → pending_inputs #2 (REVEALED → steer found!)
//!   4. re-run `run_loop(true)`: `claim_steers` promotes it → `SteerConsumed`
//!   5. follow-up LLM turn ("after-steer") → idle → Done
//!   6. second outer re-check → empty → exits
//!
//! Without P1-4 the function returns after step 2 and the steer stays stranded
//! (SteerConsumed never emitted, row still pending, "after-steer" never
//! produced) — so this test is a true regression gate.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use opencoder_core::{resolve_agent, Config, Message, Role};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_session::{run, SessionEvent, SessionState};
use opencoder_store::{
    Delivery, ImportReport, LibsqlStore, SessionEventRecord, SessionFilter, SessionInput,
    SessionListItem, SessionMeta, SessionPatch, Store, SubagentTaskRecord,
};

const SID: &str = "reabsorb-sess";
/// Number of leading `pending_inputs(Steer)` calls that see an empty set.
/// Equals the two Steer polls inside the first `run_loop` invocation:
/// `claim_steers` (turn boundary) + `has_pending_steers` (idle boundary).
const HIDE_UNTIL: u32 = 2;

fn config() -> Config {
    Config {
        model: "m/g".into(),
        ..Config::default()
    }
}

/// A text-only Completed turn (no tool calls) so the loop exits after one LLM
/// call — forcing the idle boundary and thus run_loop's return.
fn done_turn(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![],
        usage: None,
    }
}

/// Store wrapper that hides pending **Steer** inputs from the first
/// `hide_until` `pending_inputs` reads, then delegates to the real backend.
/// Queue reads and all writes pass through untouched. This deterministically
/// reproduces the "steer committed just after run_loop's last poll" window.
struct DelayedSteerStore {
    inner: Arc<LibsqlStore>,
    steer_polls: AtomicU32,
    hide_until: u32,
}

#[async_trait]
impl Store for DelayedSteerStore {
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
        // Gate only Steer visibility; Queue and everything else pass through.
        if matches!(d, Delivery::Steer) {
            let n = self.steer_polls.fetch_add(1, Ordering::SeqCst);
            if n < self.hide_until {
                return Ok(Vec::new());
            }
        }
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

/// Build the wrapped store + a session wired to it (main session: no
/// `steer_gate`, so the idle boundary does NOT settle — exactly the path P1-4
/// guards).
fn session(store: Arc<dyn Store>, mock: Arc<dyn ChatStream>) -> (tempfile::TempDir, SessionState) {
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let s = SessionState::new(SID, agent, config(), mock, dir.path().to_path_buf()).with_store(store);
    (dir, s)
}

/// Create the session row so input admission (FK) succeeds before the run.
async fn seed_session(store: &Arc<dyn Store>) {
    store
        .create_session(&SessionMeta {
            id: SID.into(),
            title: Some("t".into()),
            agent: Some("act".into()),
            model: Some("m".into()),
            created_at: 0,
            updated_at: 0,
            ..Default::default()
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn late_steer_reabsorbed_after_run_loop_returns() {
    let inner = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let store: Arc<dyn Store> = Arc::new(DelayedSteerStore {
        inner,
        steer_polls: AtomicU32::new(0),
        hide_until: HIDE_UNTIL,
    });

    // Two LLM turns: the kickoff reply and the re-absorbed steer's reply.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![done_turn("first-idle")])
            .push_script(vec![done_turn("after-steer")]),
    ) as Arc<dyn ChatStream>;

    let (_dir, mut s) = session(store.clone(), mock);
    seed_session(&store).await;

    // Admit the steer UP FRONT — it is genuinely persisted, but the wrapper
    // hides it from run_loop's first invocation so the idle boundary emits Done
    // and returns. Only P1-4's outer re-check can reveal and absorb it.
    let pk_seq = store
        .admit_input(&SessionInput {
            seq: None,
            id: "reabsorb-steer".into(),
            session_id: SID.into(),
            delivery: Delivery::Steer,
            prompt: "LATE-STEER-REABSORB".into(),
            images: Vec::new(),
            display_text: None,
            admitted_seq: 0,
            promoted_seq: None,
        })
        .await
        .unwrap();

    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev_clone = events.clone();
    run(&mut s, "kickoff".into(), move |ev| {
        ev_clone.lock().unwrap().push(ev)
    })
    .await
    .unwrap();

    // 1) The re-absorb claimed the steer → SteerConsumed carrying the row PK.
    let consumed_seqs: Vec<i64> = events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|ev| match ev {
            SessionEvent::SteerConsumed { seq, .. } => Some(*seq),
            _ => None,
        })
        .collect();
    assert_eq!(
        consumed_seqs,
        vec![pk_seq],
        "re-absorb must consume exactly the late steer (by PK)"
    );

    // 2) The steer text was promoted into history as a user message.
    let msgs = store.load_messages(SID).await.unwrap();
    let texts: Vec<String> = msgs.iter().map(|m| m.text()).collect();
    assert!(
        texts.iter().any(|t| t.contains("LATE-STEER-REABSORB")),
        "late steer must be promoted into history: {texts:?}"
    );

    // 3) The follow-up turn ran (proof the outer loop re-entered run_loop).
    assert!(
        texts.iter().any(|t| t.contains("after-steer")),
        "re-absorb must produce the follow-up turn: {texts:?}"
    );
    assert!(
        texts.iter().any(|t| t.contains("first-idle")),
        "kickoff turn must also be present: {texts:?}"
    );

    // 4) Exactly two Done events: one per run_loop invocation. Without P1-4
    //    only the first would fire and the steer would strand.
    let done_count = events
        .lock()
        .unwrap()
        .iter()
        .filter(|ev| matches!(ev, SessionEvent::Done))
        .count();
    assert_eq!(
        done_count, 2,
        "exactly two run_loop passes (kickoff + re-absorb); got {done_count}"
    );

    // 5) No stranded steer — promoted, not pending.
    let pending = store.pending_inputs(SID, Delivery::Steer).await.unwrap();
    assert!(
        pending.is_empty(),
        "late steer must not be stranded after re-absorb"
    );

    // Sanity: the steer was promoted (not still pending) in history ordering —
    // it lands as a user turn between the two assistant turns.
    let last_user_before_after = msgs
        .iter()
        .filter(|m| m.role == Role::User)
        .map(|m| m.text())
        .collect::<Vec<_>>();
    assert!(
        last_user_before_after
            .iter()
            .any(|t| t.contains("LATE-STEER-REABSORB")),
        "steer must appear as a user message"
    );
}
