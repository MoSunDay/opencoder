//! F3 (web half): bounded drain restart on initial-drain error with inputs
//! still pending (`crates/web/src/handle.rs::drain_to_completion`).
//!
//! The admit POST already answered success, so pending steer/queue rows are a
//! durable promise. When a drain `run` fails while such inputs remain, the
//! drain retries the run a bounded number of times (`MAX_DRAIN_RESTARTS = 2`)
//! instead of giving up silently — and never exceeds that budget.
//!
//! Determinism strategy: all queue inputs are seeded directly into the store
//! BEFORE `ensure_drain` (no race between admit and the first failed
//! attempt). With an always-failing client each web-drain attempt performs
//! exactly TWO LLM calls — the main `run_loop` claims one queued input and
//! fails, then the session-side F3 `reabsorb_tail` recheck claims one more
//! and fails (its error is swallowed, never masking the attempt's) — so the
//! failing stream's call counter is a stable 2·attempts observable:
//!
//!   N pending inputs ⇒ attempts = min(ceil(N/2), 1 + MAX_DRAIN_RESTARTS)
//!                    ⇒ calls = 2 × attempts (while ≥1 input stays pending)

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use opencoder_core::Config;
use opencoder_core::ContentBlock;
use opencoder_llm::{ChatRequest, ChatStream, LlmEvent};
use opencoder_store::{Delivery, EventKind, LibsqlStore, SessionInput, SessionMeta, Store};
use tokio::sync::mpsc;

/// Must mirror the web drain restart budget (`handle.rs`). Asserting the
/// literal attempt count pins the loop shape (initial + 2 restarts); copying
/// the constant would not catch a silently unbounded loop.
const MAX_DRAIN_RESTARTS: usize = 2;

// ---- failing / fail-first ChatStream impls ----

/// ChatStream whose every `chat_stream` call fails, so every LLM round errors
/// and each `run` attempt returns Err. Counts calls: the number of drain
/// attempts is observable here — the most reliable boundedness probe in this
/// harness (a store-wrapper counter would be far noisier to maintain).
struct AlwaysFailStream {
    calls: AtomicUsize,
}

impl ChatStream for AlwaysFailStream {
    fn chat_stream(&self, _req: ChatRequest) -> anyhow::Result<mpsc::Receiver<LlmEvent>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(anyhow::anyhow!("simulated LLM outage"))
    }
}

/// ChatStream that fails the first `fail_calls` calls (drain attempt #1
/// errors while queue inputs are pending → the web restart loop fires) and
/// completes every later call with a "recovered" reply, proving the retry
/// actually consumes the stranded inputs instead of just retrying forever.
struct FailFirstStream {
    fail_calls: usize,
    calls: AtomicUsize,
}

impl ChatStream for FailFirstStream {
    fn chat_stream(&self, _req: ChatRequest) -> anyhow::Result<mpsc::Receiver<LlmEvent>> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel(8);
        if n < self.fail_calls {
            return Err(anyhow::anyhow!("simulated transient LLM outage"));
        }
        tokio::spawn(async move {
            let _ = tx
                .send(LlmEvent::Completed {
                    text: "recovered".into(),
                    tool_calls: vec![],
                    usage: None,
                })
                .await;
        });
        Ok(rx)
    }
}

// ---- harness (adapted from web_steer_turn_cancel.rs) ----

/// Fresh in-memory AppState (drain tests call fns directly, no router).
async fn state() -> Arc<opencoder_web::AppState> {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    Arc::new(opencoder_web::AppState {
        client_override: None,
        store,
        workdir: std::env::temp_dir(),
        handles: opencoder_web::handle::new_handle_map(),
        nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
    })
}

/// Seed a session row (default agent "act", model "m").
async fn seed(state: &opencoder_web::AppState, sid: &str) {
    state
        .store
        .create_session(&SessionMeta {
            id: sid.to_string(),
            title: None,
            agent: Some("act".into()),
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
            plan_snapshot: None,
            plan_input_count: 0,
        })
        .await
        .unwrap();
}

/// Admit a queued input directly to the store (bypassing `admit_and_drain`)
/// so every prompt is durable BEFORE the drain starts — a deterministic input
/// supply for the bounded-restart assertions.
async fn admit_queue(state: &opencoder_web::AppState, sid: &str, prompt: &str) {
    state
        .store
        .admit_input(&SessionInput {
            seq: None,
            id: uuid::Uuid::new_v4().to_string(),
            session_id: sid.to_string(),
            delivery: Delivery::Queue,
            prompt: prompt.to_string(),
            images: vec![],
            display_text: None,
            admitted_seq: 0,
            promoted_seq: None,
        })
        .await
        .unwrap();
}

/// Start the drain (no prompt of its own) with `client`.
async fn start_drain(state: &opencoder_web::AppState, sid: &str, client: Arc<dyn ChatStream>) {
    opencoder_web::handle::ensure_drain(
        state.handles.clone(),
        state.store.clone(),
        sid,
        client,
        std::env::temp_dir(),
        Config {
            model: "m/g".into(),
            ..Default::default()
        },
    )
    .await;
}

/// Bounded poll until the drain task settles. `draining` flips false only
/// when the DrainGuard drops — after the final event flush — so once this
/// returns, the restart loop is provably finished.
async fn wait_until_not_draining(state: &opencoder_web::AppState, sid: &str) {
    let handle = {
        let map = state.handles.lock().await;
        map.get(sid)
            .cloned()
            .expect("handle present while draining")
    };
    // 600 × 10ms = 6s cap: generous vs the 2 × 250ms restart backoff.
    for _ in 0..600 {
        if !handle.draining.load(Ordering::SeqCst) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        !handle.draining.load(Ordering::SeqCst),
        "drain must end (bounded restart budget, not a hot-loop)"
    );
}

/// True once an assistant Text block containing `needle` is persisted.
async fn replied(state: &opencoder_web::AppState, sid: &str, needle: &str) -> bool {
    state
        .store
        .load_messages(sid)
        .await
        .unwrap()
        .iter()
        .flat_map(|m| m.blocks.iter())
        .any(|b| matches!(b, ContentBlock::Text { text } if text.contains(needle)))
}

// ---- tests ----

/// A drain that errors while queued inputs are still pending must retry the
/// run — bounded to `MAX_DRAIN_RESTARTS` restarts — and must leave the
/// unconsumed inputs intact (durable promise, never deleted).
#[tokio::test]
async fn drain_error_with_pending_inputs_restarts_bounded() {
    let state = state().await;
    let sid = "f3-drain-restart-bounded";
    seed(&state, sid).await;
    // Eight queued inputs — enough that the retry budget, not the input
    // supply, stops the restarts: 3 attempts × 2 inputs/attempt = 6 consumed,
    // 2 stranded past the budget.
    for i in 0..8 {
        admit_queue(&state, sid, &format!("queued #{i}")).await;
    }
    let client = Arc::new(AlwaysFailStream {
        calls: AtomicUsize::new(0),
    });
    start_drain(&state, sid, client.clone()).await;
    wait_until_not_draining(&state, sid).await;

    let attempts = 1 + MAX_DRAIN_RESTARTS;
    assert_eq!(
        client.calls.load(Ordering::SeqCst),
        2 * attempts,
        "drain must perform exactly the initial attempt plus the bounded \
         restarts (2 LLM calls per attempt): fewer means the restart never \
         fired, more means an unbounded hot-loop"
    );

    // Inputs beyond the budget stay pending — present, not deleted: the
    // durable admit-promise survives for the next drain.
    let pending = state
        .store
        .pending_inputs(sid, Delivery::Queue)
        .await
        .unwrap();
    assert_eq!(pending.len(), 2, "inputs past the budget stay pending");

    // Every failed LLM round broadcasts + persists an Error event, so the
    // retry budget is observable in the durable event log too.
    let errors = state
        .store
        .events_after(sid, 0)
        .await
        .unwrap()
        .iter()
        .filter(|r| r.kind == EventKind::Error)
        .count();
    assert_eq!(errors, 2 * attempts);
}

/// The retry is not a zombie: once the LLM recovers, the restarted drain
/// consumes the stranded pending inputs to completion and ends cleanly.
#[tokio::test]
async fn drain_restart_recovers_stranded_pending_inputs() {
    let state = state().await;
    let sid = "f3-drain-restart-recovers";
    seed(&state, sid).await;
    // Four queued inputs; the first two LLM calls fail so attempt #1 (main
    // run_loop + one reabsorb recheck) errors with 2 inputs still pending —
    // the web restart fires — and every later call completes.
    for i in 0..4 {
        admit_queue(&state, sid, &format!("queued #{i}")).await;
    }
    let client = Arc::new(FailFirstStream {
        fail_calls: 2,
        calls: AtomicUsize::new(0),
    });
    start_drain(&state, sid, client.clone()).await;
    wait_until_not_draining(&state, sid).await;

    assert!(
        client.calls.load(Ordering::SeqCst) >= 4,
        "the restarted drain must run the recovered LLM turns"
    );
    assert!(
        replied(&state, sid, "recovered").await,
        "restarted drain must consume the stranded queue inputs"
    );
    let pending = state
        .store
        .pending_inputs(sid, Delivery::Queue)
        .await
        .unwrap();
    assert!(pending.is_empty(), "recovered drain drains the queue fully");
}
