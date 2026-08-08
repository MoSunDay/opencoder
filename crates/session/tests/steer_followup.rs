//! P2 functional tests for the steer/queue drain semantics.
//!
//! Contracts (mirroring opencoder's two-tier delivery model):
//! - steer_promotes_at_turn_boundary: a steer admitted during a run is
//!   appended to history at the next turn boundary
//! - multiple_steers_one_boundary_promoted_once: N steers at one boundary are
//!   all promoted at that boundary
//! - queue_drains_all_fifo_at_idle: queued inputs never interrupt a running
//!   turn; at idle all are drained FIFO until the queue is empty
//! - durable_pending_survives_to_next_drain: an admitted-but-unpromoted steer
//!   persists in the store until a drain claims it
//! - no_store_no_steering: without a store the runner behaves classically
//!
//! These drive the real runner loop with a MockChatClient that emits a long
//! tool-calling sequence so the drain has multiple turn boundaries to absorb
//! steers at.

use std::sync::Arc;
use std::time::Duration;

use opencoder_core::{resolve_agent, Config, ContentBlock, Message, Role};
use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::{run, SessionEvent, SessionState, SubagentSteerGate};
use opencoder_store::{Delivery, LibsqlStore, SessionInput, Store};

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

fn config() -> Config {
    Config {
        model: "m/g".into(),
        ..Config::default()
    }
}

/// A turn that calls `bash` (so the loop continues), carrying `n` in usage.
fn bash_turn(n: u32) -> LlmEvent {
    LlmEvent::Completed {
        text: format!("turn-{n}"),
        tool_calls: vec![CompletedToolCall {
            id: format!("tu{n}"),
            name: "bash".into(),
            input: serde_json::json!({"command": "true"}),
        }],
        usage: Some(Usage {
            input_tokens: 10 * n as u64,
            output_tokens: 1,
            total_tokens: 10 * n as u64 + 1,
            ..Default::default()
        }),
    }
}

fn done_turn(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![],
        usage: None,
    }
}

fn session(store: Arc<dyn Store>, mock: Arc<dyn ChatStream>) -> (tempfile::TempDir, SessionState) {
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let s = SessionState::new(
        "drain-sess",
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
        .create_session(&opencoder_store::SessionMeta {
            id: "drain-sess".into(),
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

/// Create the session row (parameterized id) so input admission (FK) succeeds.
async fn seed_session_id(store: &Arc<dyn Store>, id: &str) {
    store
        .create_session(&opencoder_store::SessionMeta {
            id: id.into(),
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

/// Admit a steer input; returns the row PK seq (`admit_input`'s return value).
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

#[tokio::test]
async fn steer_promotes_at_turn_boundary() {
    let store = mem_store().await;
    // 3 bash turns then done. After turn 2 starts, we admit a steer that must
    // land before turn 3 — but since the loop is synchronous per turn, we admit
    // the steer UP FRONT; it should be promoted at the FIRST boundary (top of
    // turn 2) because it's pending from the start.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![bash_turn(1)])
            .push_script(vec![bash_turn(2)])
            .push_script(vec![bash_turn(3)])
            .push_script(vec![done_turn("final")]),
    ) as Arc<dyn ChatStream>;

    let (_dir, mut s) = session(store.clone(), mock.clone());
    // admit a steer BEFORE the run starts; the drain claims it at the first
    // turn boundary (top of iteration 2, since iteration 1 has no prior boundary).
    seed_session(&store).await;
    store
        .admit_input(&SessionInput {
            seq: None,
            id: "in-1".into(),
            session_id: "drain-sess".into(),
            delivery: Delivery::Steer,
            prompt: "STEER-MARKER".into(),
            images: Vec::new(),
            display_text: None,
            admitted_seq: 0,
            promoted_seq: None,
        })
        .await
        .unwrap();

    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev_clone = events.clone();
    run(&mut s, "kickoff".into(), move |ev| {
        ev_clone.lock().unwrap().push(ev)
    })
    .await
    .unwrap();

    // The steer text must appear in the persisted history (promoted into it).
    let msgs = store.load_messages("drain-sess").await.unwrap();
    let texts: Vec<String> = msgs.iter().map(|m| m.text()).collect();
    assert!(
        texts.iter().any(|t| t.contains("STEER-MARKER")),
        "steer must be promoted into history: {texts:?}"
    );
    // ...and the steer must no longer be pending.
    let pending = store
        .pending_inputs("drain-sess", Delivery::Steer)
        .await
        .unwrap();
    assert!(pending.is_empty(), "steer consumed");
}

#[tokio::test]
async fn multiple_steers_at_one_boundary_promoted_once() {
    let store = mem_store().await;
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![bash_turn(1)])
            .push_script(vec![done_turn("done")]),
    ) as Arc<dyn ChatStream>;
    let (_dir, mut s) = session(store.clone(), mock);
    for i in 0..3u32 {
        seed_session(&store).await;
        store
            .admit_input(&SessionInput {
                seq: None,
                id: format!("ms-{i}"),
                session_id: "drain-sess".into(),
                delivery: Delivery::Steer,
                prompt: format!("multi-{i}"),
                images: Vec::new(),
                display_text: None,
                admitted_seq: 0,
                promoted_seq: None,
            })
            .await
            .unwrap();
    }
    run(&mut s, "go".into(), |_| {}).await.unwrap();
    let msgs = store.load_messages("drain-sess").await.unwrap();
    let promoted_count = msgs
        .iter()
        .filter(|m| m.role == Role::User && m.text().starts_with("multi-"))
        .count();
    assert_eq!(
        promoted_count, 3,
        "all 3 steers promoted at the same boundary"
    );
}

#[tokio::test]
async fn queue_drains_all_fifo_in_single_run_then_done() {
    let store = mem_store().await;
    // A single run drains the entire queue FIFO: "start" turn -> idle ->
    // consume QUEUE-1 -> turn -> idle -> consume QUEUE-2 -> turn -> idle ->
    // Done (queue empty). Steer is the clear-all path; queue is drained one
    // item per turn boundary until empty within a single run.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![done_turn("t1")])
            .push_script(vec![done_turn("t2")])
            .push_script(vec![done_turn("t3")]),
    ) as Arc<dyn ChatStream>;
    let (_dir, mut s) = session(store.clone(), mock);
    for i in 1..=2u32 {
        seed_session(&store).await;
        store
            .admit_input(&SessionInput {
                seq: None,
                id: format!("q-{i}"),
                session_id: "drain-sess".into(),
                delivery: Delivery::Queue,
                prompt: format!("QUEUE-{i}"),
                images: Vec::new(),
                display_text: None,
                admitted_seq: 0,
                promoted_seq: None,
            })
            .await
            .unwrap();
    }
    run(&mut s, "start".into(), |_| {}).await.unwrap();
    let msgs = store.load_messages("drain-sess").await.unwrap();
    let queued_promoted: Vec<String> = msgs
        .iter()
        .filter(|m| m.role == Role::User && m.text().starts_with("QUEUE-"))
        .map(|m| m.text())
        .collect();
    assert_eq!(
        queued_promoted.len(),
        2,
        "single run drains both queued items in FIFO order"
    );
    assert_eq!(queued_promoted[0], "QUEUE-1");
    assert_eq!(queued_promoted[1], "QUEUE-2");
    let still_pending = store
        .pending_inputs("drain-sess", Delivery::Queue)
        .await
        .unwrap();
    assert!(
        still_pending.is_empty(),
        "queue fully drained after single run"
    );
}

#[tokio::test]
async fn durable_pending_input_survives_until_drain() {
    let store = mem_store().await;
    // admit a steer but DON'T run; it must sit pending.
    seed_session(&store).await;
    store
        .admit_input(&SessionInput {
            seq: None,
            id: "p-1".into(),
            session_id: "drain-sess".into(),
            delivery: Delivery::Steer,
            prompt: "waiting".into(),
            images: Vec::new(),
            display_text: None,
            admitted_seq: 0,
            promoted_seq: None,
        })
        .await
        .unwrap();
    // simulate "process restart" by checking pending on a fresh handle
    let pending = store
        .pending_inputs("drain-sess", Delivery::Steer)
        .await
        .unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].prompt, "waiting");
    assert!(pending[0].promoted_seq.is_none(), "not yet promoted");
}

#[tokio::test]
async fn no_store_attached_behaves_classically() {
    // No store → no steering. A single done turn finishes immediately.
    let mock =
        Arc::new(MockChatClient::new().push_script(vec![done_turn("ok")])) as Arc<dyn ChatStream>;
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let mut s = SessionState::new("classic", agent, config(), mock, dir.path().to_path_buf());
    let mut saw_done = false;
    run(&mut s, "hi".into(), |ev| {
        if matches!(ev, SessionEvent::Done) {
            saw_done = true;
        }
    })
    .await
    .unwrap();
    assert!(saw_done);
    // sanity: keep imports honest
    let _ = ContentBlock::text("x");
    let _: Message = Message::user("id", "x");
    let _ = Duration::from_millis(0);
}

/// Regression: `SteerConsumed` must carry the row PK seq (what `admit_input`
/// returns and the TUI stores), NOT the per-session `admitted_seq`. With the
/// bug the event carried `admitted_seq`, so the TUI's `retain(|(s,_)| s != seq)`
/// compared a stored PK against an `admitted_seq` -> never matched -> the steer
/// row lingered until `Done`. To make PK != admitted_seq we first admit noise
/// inputs to a DIFFERENT session: those bump the global autoincrement PK but
/// not `drain-sess`'s per-session `admitted_seq`.
#[tokio::test]
async fn steer_consumed_carries_pk_seq_not_admitted_seq() {
    let store = mem_store().await;
    // Noise in another session: advances the global PK counter only.
    seed_session_id(&store, "other").await;
    for i in 0..3u32 {
        admit_steer(&store, "other", &format!("noise-{i}"), &format!("n{i}")).await;
    }

    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![bash_turn(1)])
            .push_script(vec![done_turn("done")]),
    ) as Arc<dyn ChatStream>;
    let (_dir, mut s) = session(store.clone(), mock);
    seed_session(&store).await;
    let pk_seq = admit_steer(&store, "drain-sess", "sc-1", "STEER-SEQ").await;

    // Sanity: the PK must differ from admitted_seq (1) for this test to mean
    // anything. 3 noise rows -> this steer is row 4 globally, admitted_seq 1.
    assert_ne!(
        pk_seq, 1,
        "test setup must diverge PK from admitted_seq; got pk={pk_seq}"
    );

    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev_clone = events.clone();
    run(&mut s, "kickoff".into(), move |ev| {
        ev_clone.lock().unwrap().push(ev)
    })
    .await
    .unwrap();

    let consumed: Vec<i64> = events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|ev| match ev {
            SessionEvent::SteerConsumed { seq, .. } => Some(*seq),
            _ => None,
        })
        .collect();
    assert_eq!(
        consumed,
        vec![pk_seq],
        "SteerConsumed must carry the admit_input PK seq, not admitted_seq"
    );
}

/// Multiple steers promoted at one boundary must each emit a `SteerConsumed`
/// carrying the correct distinct PK seq, in `admitted_seq ASC` order. Guards
/// the zip alignment between `pending_inputs` and `promote_inputs` returns.
#[tokio::test]
async fn multiple_steers_consumed_each_carries_distinct_pk_seq() {
    let store = mem_store().await;
    // Noise in another session so PKs diverge from admitted_seqs.
    seed_session_id(&store, "other").await;
    for i in 0..2u32 {
        admit_steer(&store, "other", &format!("noise-{i}"), &format!("n{i}")).await;
    }

    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![bash_turn(1)])
            .push_script(vec![done_turn("done")]),
    ) as Arc<dyn ChatStream>;
    let (_dir, mut s) = session(store.clone(), mock);
    seed_session(&store).await;
    // 3 steers: PKs 3,4,5 ; admitted_seq 1,2,3.
    let pk0 = admit_steer(&store, "drain-sess", "ms-0", "S0").await;
    let pk1 = admit_steer(&store, "drain-sess", "ms-1", "S1").await;
    let pk2 = admit_steer(&store, "drain-sess", "ms-2", "S2").await;
    let expected = vec![pk0, pk1, pk2];
    // Sanity: PKs are strictly increasing and distinct from admitted_seqs.
    assert!(pk0 < pk1 && pk1 < pk2, "PKs must be distinct/increasing");
    assert_eq!(pk0, 3, "2 noise rows -> first drain-sess steer is row 3");

    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev_clone = events.clone();
    run(&mut s, "kickoff".into(), move |ev| {
        ev_clone.lock().unwrap().push(ev)
    })
    .await
    .unwrap();

    let consumed: Vec<i64> = events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|ev| match ev {
            SessionEvent::SteerConsumed { seq, .. } => Some(*seq),
            _ => None,
        })
        .collect();
    assert_eq!(
        consumed, expected,
        "each SteerConsumed must carry the correct PK in admitted_seq order"
    );
}

// ---- Late-steer-at-idle-boundary regression ----
//
// A steer admitted during a text-only turn (after the top-of-loop
// claim_steers, before the idle boundary) used to be stranded: the queue is
// empty so the runner emitted Done and exited, leaving the steer row with
// promoted_seq = NULL. The idle-boundary peek now catches it.

use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use opencoder_llm::ChatRequest;
use tokio::sync::mpsc;

/// Wraps a real ChatStream. On the FIRST text-only (no tool_calls) Completed
/// event it forwards, it admits a Steer into the store BEFORE forwarding the
/// event. Since the runner cannot reach the idle boundary until it receives
/// that event (and the channel then closes), the admit is guaranteed complete
/// by the time the idle boundary runs -- deterministically reproducing the race.
struct SteerOnIdle {
    inner: Arc<dyn ChatStream>,
    store: Arc<dyn Store>,
    session_id: String,
    admitted: Arc<AtomicBool>,
    gate: Arc<SubagentSteerGate>,
}

impl ChatStream for SteerOnIdle {
    fn chat_stream(&self, req: ChatRequest) -> Result<mpsc::Receiver<LlmEvent>> {
        let mut rx = self.inner.chat_stream(req)?;
        let (tx, out_rx) = mpsc::channel::<LlmEvent>(128);
        let store = self.store.clone();
        let sid = self.session_id.clone();
        // Share self.admitted across ALL chat_stream calls so the steer is
        // admitted exactly once (on the first text-only Completed), not on
        // every text-only turn.
        let flag = self.admitted.clone();
        let gate = self.gate.clone();
        tokio::spawn(async move {
            while let Some(ev) = rx.recv().await {
                if let LlmEvent::Completed { ref tool_calls, .. } = ev {
                    if tool_calls.is_empty() && !flag.swap(true, Ordering::SeqCst) {
                        let reservation = gate.reserve().expect("child gate must be open");
                        let admitted = store
                            .admit_input(&SessionInput {
                                seq: None,
                                id: "late-steer".into(),
                                session_id: sid.clone(),
                                delivery: Delivery::Steer,
                                prompt: "LATE-STEER".into(),
                                images: Vec::new(),
                                display_text: None,
                                admitted_seq: 0,
                                promoted_seq: None,
                            })
                            .await;
                        assert!(admitted.is_ok(), "steer admission must succeed");
                        assert!(reservation.commit(), "live child must accept steer");
                    }
                }
                if tx.send(ev).await.is_err() {
                    break;
                }
            }
        });
        Ok(out_rx)
    }

    fn backend(&self) -> &'static str {
        self.inner.backend()
    }
}

#[tokio::test]
async fn late_steer_absorbed_at_idle_boundary() {
    let store = mem_store().await;
    let gate = SubagentSteerGate::new();
    // Turn 1 -> text-only "first-idle" (the wrapper admits LATE-STEER just
    // before forwarding this). Turn 2 -> text-only "after-steer" (the
    // follow-up turn the absorbed steer triggers).
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![done_turn("first-idle")])
            .push_script(vec![done_turn("after-steer")]),
    ) as Arc<dyn ChatStream>;

    let wrapper = Arc::new(SteerOnIdle {
        inner: mock,
        store: store.clone(),
        session_id: "drain-sess".into(),
        admitted: std::sync::Arc::new(AtomicBool::new(false)),
        gate: gate.clone(),
    }) as Arc<dyn ChatStream>;

    let (_dir, mut s) = session(store.clone(), wrapper);
    s.steer_gate = Some(gate);
    seed_session(&store).await;

    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev_clone = events.clone();
    run(&mut s, "kickoff".into(), move |ev| {
        ev_clone.lock().unwrap().push(ev)
    })
    .await
    .unwrap();

    // The late steer must have been absorbed: SteerConsumed emitted...
    let steer_consumed = events
        .lock()
        .unwrap()
        .iter()
        .any(|ev| matches!(ev, SessionEvent::SteerConsumed { .. }));
    assert!(
        steer_consumed,
        "late steer must be consumed (SteerConsumed)"
    );

    // ...its text promoted into history...
    let msgs = store.load_messages("drain-sess").await.unwrap();
    let texts: Vec<String> = msgs.iter().map(|m| m.text()).collect();
    assert!(
        texts.iter().any(|t| t.contains("LATE-STEER")),
        "late steer must be promoted into history: {texts:?}"
    );

    // ...and no longer pending (promoted_seq set, not stranded).
    let pending = store
        .pending_inputs("drain-sess", Delivery::Steer)
        .await
        .unwrap();
    assert!(pending.is_empty(), "late steer must not be stranded");
}
