//! Zero-resubmit (web half): a failed drain NEVER restarts and NEVER
//! auto-resubmits pending inputs (`crates/web/src/handle.rs::drain_to_completion`).
//!
//! The admit POST's pending steer/queue rows remain a durable promise, but it
//! is honored by the NEXT successful drain (e.g. a later prompt admit) — not
//! by invisible retries inside the failing one. The contract pinned here:
//!
//! * an always-failing LLM drives EXACTLY ONE `run` attempt per drain — the
//!   failing stream's call counter must settle at 1, no bounded restart loop;
//! * the drain consumes only the head item (its failed turn), every tail item
//!   stays PENDING in the store, never deleted;
//! * the Error is both broadcast on the session's SSE channel and persisted,
//!   so consumers see the failure normally;
//! * a subsequent drain with a healthy client consumes the stranded items.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use opencoder_core::Config;
use opencoder_core::ContentBlock;
use opencoder_llm::{ChatRequest, ChatStream, LlmEvent};
use opencoder_store::{Delivery, EventKind, LibsqlStore, SessionInput, SessionMeta, Store};
use tokio::sync::mpsc;

// ---- failing / fail-first ChatStream impls ----

/// ChatStream whose every `chat_stream` call fails. Counts calls: with the
/// restart loop removed, one drain must settle at EXACTLY one call.
struct AlwaysFailStream {
    calls: AtomicUsize,
}

impl AlwaysFailStream {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }
}

impl ChatStream for AlwaysFailStream {
    fn chat_stream(&self, _req: ChatRequest) -> anyhow::Result<mpsc::Receiver<LlmEvent>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(anyhow::anyhow!("simulated LLM outage"))
    }
}

/// Fails the first `fail_calls` calls, then completes every later call with a
/// "recovered" reply — models the LLM coming back between two drains, so the
/// next (explicit) drain consumes the stranded pending inputs.
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
        controls: Arc::new(opencoder_web::control_state::ControlHub::new()),
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
        })
        .await
        .unwrap();
}

/// Admit a queued input directly to the store (bypassing `admit_and_drain`)
/// so every prompt is durable BEFORE the drain starts — a deterministic input
/// supply for the no-resubmit assertions.
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
/// returns, the drain is provably finished (a single attempt, no restart).
async fn wait_until_not_draining(state: &opencoder_web::AppState, sid: &str) {
    let handle = {
        let map = state.handles.lock().await;
        map.get(sid)
            .cloned()
            .expect("handle present while draining")
    };
    for _ in 0..600 {
        if !handle.draining.load(Ordering::SeqCst) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("drain must end (single attempt, no restart loop)");
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

/// A drain whose LLM fails must run EXACTLY ONE attempt (no bounded restart),
/// broadcast + persist the Error, consume only the head queued item, and
/// leave every tail item pending — visible, recoverable, never deleted.
#[tokio::test]
async fn drain_error_never_restarts_and_keeps_inputs_pending() {
    let state = state().await;
    let sid = "f3-drain-no-restart";
    seed(&state, sid).await;
    for i in 0..3 {
        admit_queue(&state, sid, &format!("queued #{i}")).await;
    }

    // Subscribe to the SSE broadcast BEFORE the drain so the live Error
    // passthrough is observable (the events endpoint forwards this channel).
    let mut sse_rx = {
        let mut map = state.handles.lock().await;
        let handle = map.get(sid).cloned().unwrap_or_else(|| {
            let h = opencoder_web::handle::SessionHandle::new();
            map.insert(sid.to_string(), h.clone());
            h
        });
        handle.tx.subscribe()
    };

    let client = Arc::new(AlwaysFailStream::new());
    start_drain(&state, sid, client.clone()).await;
    wait_until_not_draining(&state, sid).await;

    // EXACTLY one LLM request: the failed turn of the head item. A restart
    // loop (or a session-side error re-absorb) would make this 2+.
    assert_eq!(
        client.calls.load(Ordering::SeqCst),
        1,
        "a failed drain must settle after a single attempt"
    );

    // The Error is broadcast live on the session's SSE channel.
    let mut saw_error_sse = false;
    while let Ok(evt) = sse_rx.try_recv() {
        if evt.kind == "error" {
            saw_error_sse = true;
        }
    }
    assert!(
        saw_error_sse,
        "the failed run's Error must reach the SSE stream"
    );

    // ... and persisted, so late subscribers replay it too.
    let errors = state
        .store
        .events_after(sid, 0)
        .await
        .unwrap()
        .iter()
        .filter(|r| r.kind == EventKind::Error)
        .count();
    assert_eq!(
        errors, 1,
        "exactly one failed attempt ⇒ exactly one Error event"
    );

    // Head item consumed (its user message persisted); tails stay pending.
    assert!(
        state
            .store
            .load_messages(sid)
            .await
            .unwrap()
            .iter()
            .filter(|m| m.role == opencoder_core::Role::User)
            .flat_map(|m| m.blocks.iter())
            .any(|b| matches!(b, ContentBlock::Text { text } if text.contains("queued #0"))),
        "the consumed head item must be recorded as a user message"
    );
    let pending = state
        .store
        .pending_inputs(sid, Delivery::Queue)
        .await
        .unwrap();
    let texts: Vec<&str> = pending.iter().map(|i| i.prompt.as_str()).collect();
    assert_eq!(
        texts,
        vec!["queued #1", "queued #2"],
        "tail inputs must stay pending after the failed drain: {texts:?}"
    );
}

/// The stranded inputs are not abandoned: once the LLM recovers, the NEXT
/// drain (an explicit new prompt's drain) consumes them to completion — the
/// durable admit-promise is honored across drain boundaries, not by retries
/// inside the failing drain.
#[tokio::test]
async fn next_drain_consumes_stranded_pending_inputs() {
    let state = state().await;
    let sid = "f3-drain-next-consumes";
    seed(&state, sid).await;
    for i in 0..3 {
        admit_queue(&state, sid, &format!("queued #{i}")).await;
    }

    // Drain #1: the first LLM call fails; the tail items survive pending.
    let client = Arc::new(FailFirstStream {
        fail_calls: 1,
        calls: AtomicUsize::new(0),
    });
    start_drain(&state, sid, client.clone()).await;
    wait_until_not_draining(&state, sid).await;
    assert_eq!(client.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        state
            .store
            .pending_inputs(sid, Delivery::Queue)
            .await
            .unwrap()
            .len(),
        2,
        "the failed drain leaves the tail items pending"
    );

    // Drain #2 (healthy LLM): consumes the stranded items and ends cleanly.
    start_drain(&state, sid, client.clone()).await;
    wait_until_not_draining(&state, sid).await;
    assert!(
        replied(&state, sid, "recovered").await,
        "the next drain must consume the stranded queue inputs"
    );
    assert!(
        state
            .store
            .pending_inputs(sid, Delivery::Queue)
            .await
            .unwrap()
            .is_empty(),
        "the recovered drain drains the queue fully"
    );
}
