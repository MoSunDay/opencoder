//! Regression guard: TUI queue admission must stay OFF the event loop.
//!
//! The bug this pins: `KeyAction::Queue` used to `.await store.admit_input`
//! inline in the select-loop key branch. `admit_input` contends on the
//! store-wide db_lock with the running turn's message flusher, subagent
//! flushers and queue claims, so a Tab-queue submit during a busy turn froze
//! rendering and input for the whole queue-wait (visible as a multi-frame
//! stutter, sometimes a full second of dead UI).
//!
//! `queue_admitter::spawn_admitter` owns that wait in a separate actor; the
//! UI keeps an optimistic temp row (negative seq) and reconciles it when the
//! completion lands ([`queue_admitter::reconcile_ok`]). These tests drive a
//! select-loop shaped exactly like `app.rs` (input first, then
//! `admit_done_rx` behind the `alive` guard, then tickers) against a store
//! whose `admit_input` deliberately blocks, and prove:
//!   1. frame ticks and key events keep flowing while the admit is blocked,
//!      and the temp row reconciles to the real seq;
//!   2. a consume that raced ahead of the completion drops the temp row
//!      instead of resurrecting it;
//!   3. a second submit lines up behind a blocked first one, FIFO, with both
//!      completions reconciling in arrival order.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use opencoder_core::Message;
use opencoder_store::{
    Delivery, LibsqlStore, SessionEventRecord, SessionFilter, SessionInput, SessionListItem,
    SessionMeta, SessionPatch, Store, SubagentTaskRecord,
};
use opencoder_tui::queue_admitter::{self, AdmitDone, AdmitReconcile, AdmitUiState};

/// A Store wrapper that delegates everything to an inner LibsqlStore EXCEPT
/// `admit_input`, which sleeps `admit_delay` first — simulating the db_lock
/// contention the actor exists to absorb. The atomics let the test observe
/// whether the (blocking) store call was actually entered/finished.
struct DelayStore {
    inner: Arc<LibsqlStore>,
    admit_delay: Duration,
    admit_entered: AtomicBool,
    admit_finished: AtomicBool,
}

impl DelayStore {
    fn new(inner: Arc<LibsqlStore>, admit_delay: Duration) -> Self {
        Self {
            inner,
            admit_delay,
            admit_entered: AtomicBool::new(false),
            admit_finished: AtomicBool::new(false),
        }
    }
}

#[async_trait]
impl Store for DelayStore {
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
    async fn admit_input(&self, input: &SessionInput) -> Result<i64> {
        self.admit_entered.store(true, Ordering::SeqCst);
        tokio::time::sleep(self.admit_delay).await;
        let r = self.inner.admit_input(input).await;
        self.admit_finished.store(true, Ordering::SeqCst);
        r
    }
    async fn pending_inputs(&self, sid: &str, d: Delivery) -> Result<Vec<SessionInput>> {
        self.inner.pending_inputs(sid, d).await
    }
    async fn promote_inputs(&self, sid: &str, up: i64, d: Delivery) -> Result<Vec<i64>> {
        self.inner.promote_inputs(sid, up, d).await
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
    async fn append_events(&self, ev: &[SessionEventRecord]) -> Result<Vec<i64>> {
        self.inner.append_events(ev).await
    }
    async fn events_after(&self, sid: &str, s: i64) -> Result<Vec<SessionEventRecord>> {
        self.inner.events_after(sid, s).await
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
    async fn list_subagent_tasks(&self, pid: &str) -> Result<Vec<SubagentTaskRecord>> {
        self.inner.list_subagent_tasks(pid).await
    }
    async fn get_subagent_task(&self, id: &str) -> Result<Option<SubagentTaskRecord>> {
        self.inner.get_subagent_task(id).await
    }
    async fn cancel_subagent_task(&self, id: &str) -> Result<()> {
        self.inner.cancel_subagent_task(id).await
    }
}

/// A Delivery::Queue SessionInput exactly like `app_helpers::mk_input` builds
/// for a plain queue submit (no display override, no images).
fn q_input(session_id: &str, prompt: &str) -> SessionInput {
    SessionInput {
        seq: None,
        id: "x".into(),
        session_id: session_id.into(),
        delivery: Delivery::Queue,
        prompt: prompt.into(),
        images: Vec::new(),
        display_text: None,
        admitted_seq: 0,
        promoted_seq: None,
    }
}

/// Unwrap a completion that must have succeeded and return its real store seq.
fn ok_seq(done: &AdmitDone) -> i64 {
    *done.result.as_ref().expect("admit must succeed")
}

/// In-memory store + session + DelayStore wrapper, shared by all three tests.
async fn setup(admit_delay: Duration) -> Arc<DelayStore> {
    let inner = Arc::new(LibsqlStore::open_memory().await.unwrap());
    inner
        .create_session(&SessionMeta {
            id: "s1".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    Arc::new(DelayStore::new(inner, admit_delay))
}

#[tokio::test]
async fn blocked_admit_does_not_stall_ui_loop() {
    let wrapper = setup(Duration::from_millis(120)).await;
    let store: Arc<dyn Store> = wrapper.clone();
    let (tx, mut done_rx) = queue_admitter::spawn_admitter(store);
    let mut st = AdmitUiState::default();
    let mut queue_items: Vec<(i64, String)> = vec![];
    let mut pending: Vec<(String, String)> = vec![];

    // Optimistic submit: strictly non-blocking, temp row visible at once.
    assert!(queue_admitter::submit(
        &tx,
        &mut st,
        &mut queue_items,
        &mut pending,
        q_input("s1", "first"),
        "first".into()
    ));
    assert_eq!(queue_items, vec![(-1, "first".to_string())]);
    assert_eq!(st.inflight.len(), 1);

    // Mini event loop shaped exactly like app.rs: biased select with key
    // input first, the admit completion behind the `alive` guard, then the
    // frame ticker.
    let (key_tx, mut key_rx) = tokio::sync::mpsc::channel::<()>(8);
    let mut ticker = tokio::time::interval(Duration::from_millis(10));
    let mut ticks = 0u32;
    let mut keys = 0u32;
    let mut completion: Option<AdmitDone> = None;
    let mut alive = true;
    // Arm one "keypress" 30ms in, like a user typing while the admit blocks.
    // `key_keep` mirrors the real TUI's input-collector thread: it keeps the
    // channel OPEN after the key (a closed channel is permanently ready with
    // None, which the biased select would spin on and starve the ticker).
    let key_keep = key_tx.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(30)).await;
        let _ = key_tx.send(()).await;
    });
    let _ = &key_keep;
    let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
    while completion.is_none() && tokio::time::Instant::now() < deadline {
        tokio::select! {
            biased;
            k = key_rx.recv() => { if k.is_some() { keys += 1; } }
            d = done_rx.recv(), if alive => {
                match d {
                    Some(done) => completion = Some(done),
                    None => alive = false,
                }
            }
            _ = ticker.tick() => { ticks += 1; }
        }
    }

    assert!(
        wrapper.admit_entered.load(Ordering::SeqCst),
        "admit must actually be in the (blocked) store"
    );
    assert!(
        ticks >= 5,
        "frame ticks kept flowing while admit held the store (got {ticks})"
    );
    assert!(
        keys >= 1,
        "a keypress during the blocked admit was serviced (got {keys})"
    );
    let done = completion.expect("completion within 500ms deadline");
    let real = ok_seq(&done);
    assert!(real > 0, "store returned a real seq, got {real}");
    assert!(
        wrapper.admit_finished.load(Ordering::SeqCst),
        "the delayed admit finished before the completion was sent"
    );

    // The optimistic temp row reconciles to the real seq, in place.
    assert_eq!(
        queue_admitter::reconcile_ok(&mut queue_items, &st.consumed, -1, real),
        AdmitReconcile::Replaced
    );
    assert_eq!(queue_items, vec![(real, "first".to_string())]);
    assert_eq!(
        wrapper.pending_inputs("s1", Delivery::Queue).await.unwrap().len(),
        1,
        "the admitted row is durably queued in the store"
    );
}

#[tokio::test]
async fn consumed_before_completion_does_not_resurrect() {
    let wrapper = setup(Duration::from_millis(30)).await;
    let store: Arc<dyn Store> = wrapper.clone();
    let (tx, mut done_rx) = queue_admitter::spawn_admitter(store);
    let mut st = AdmitUiState::default();
    let mut queue_items: Vec<(i64, String)> = vec![];
    let mut pending: Vec<(String, String)> = vec![];

    assert!(queue_admitter::submit(
        &tx,
        &mut st,
        &mut queue_items,
        &mut pending,
        q_input("s1", "first"),
        "first".into()
    ));
    assert_eq!(queue_items, vec![(-1, "first".to_string())]);

    // Wait for the completion, but simulate the drain having claimed + run
    // the real seq BEFORE the UI folded the completion in (a QueueConsumed
    // event can beat the actor's done message under load).
    let done = tokio::time::timeout(Duration::from_secs(2), done_rx.recv())
        .await
        .expect("completion within 2s")
        .expect("actor alive");
    let real = ok_seq(&done);
    assert!(real > 0);

    queue_admitter::note_consumed(&mut st, real);
    assert_eq!(
        queue_admitter::reconcile_ok(&mut queue_items, &st.consumed, -1, real),
        AdmitReconcile::DroppedConsumed,
        "the drain already consumed this seq: the temp row must drop, not resurrect"
    );
    assert!(queue_items.is_empty(), "queue mirror must not resurrect the row");
}

#[tokio::test]
async fn second_submit_lines_up_behind_blocked_first() {
    let wrapper = setup(Duration::from_millis(120)).await;
    let store: Arc<dyn Store> = wrapper.clone();
    let (tx, mut done_rx) = queue_admitter::spawn_admitter(store);
    let mut st = AdmitUiState::default();
    let mut queue_items: Vec<(i64, String)> = vec![];
    let mut pending: Vec<(String, String)> = vec![];

    // Two submits on the same tick: neither awaits the (blocked) store.
    assert!(queue_admitter::submit(
        &tx,
        &mut st,
        &mut queue_items,
        &mut pending,
        q_input("s1", "first"),
        "first".into()
    ));
    assert!(queue_admitter::submit(
        &tx,
        &mut st,
        &mut queue_items,
        &mut pending,
        q_input("s1", "second"),
        "second".into()
    ));
    assert_eq!(
        queue_items,
        vec![(-1, "first".to_string()), (-2, "second".to_string())]
    );
    assert_eq!(st.inflight.len(), 2, "both admits are in flight");

    // The actor is serial FIFO: 2 x 120ms ≈ 240ms; 3s deadline is generous.
    let mut completions: Vec<AdmitDone> = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while completions.len() < 2 && tokio::time::Instant::now() < deadline {
        match tokio::time::timeout_at(deadline, done_rx.recv()).await {
            Ok(Some(done)) => completions.push(done),
            Ok(None) => break, // actor gone
            Err(_) => break,  // deadline hit
        }
    }
    assert_eq!(completions.len(), 2, "both admits must complete");
    let (r1, r2) = (ok_seq(&completions[0]), ok_seq(&completions[1]));
    assert!(
        r1 > 0 && r2 > 0 && r1 < r2,
        "FIFO processing must give the first submit a lower real seq: {r1} vs {r2}"
    );

    // Reconcile in arrival order; each temp row rewrites in place.
    assert_eq!(
        queue_admitter::reconcile_ok(&mut queue_items, &st.consumed, -1, r1),
        AdmitReconcile::Replaced
    );
    assert_eq!(
        queue_admitter::reconcile_ok(&mut queue_items, &st.consumed, -2, r2),
        AdmitReconcile::Replaced
    );
    assert_eq!(
        queue_items,
        vec![(r1, "first".to_string()), (r2, "second".to_string())]
    );
    assert_eq!(
        wrapper.pending_inputs("s1", Delivery::Queue).await.unwrap().len(),
        2,
        "both rows are durably queued in submit order"
    );
}
