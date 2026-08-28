//! Integration test: a `Delivery::Steer` prompt arriving mid-drain fires
//! `turn_cancel` on the web `SessionHandle`, interrupting the in-flight LLM
//! turn so the loop can absorb the new steer.
//!
//! Mirrors the runtime contract exercised by
//! `crates/session/tests/parent_turn_cancel_steer.rs` but through the real web
//! `admit_and_drain` entrypoint: a `BlockingFirstStream` hangs the first
//! `chat_stream` call (drain enters and gets stuck in the LLM turn); a second
//! `Steer` admit while `draining` is true takes the else-branch and calls
//! `opencoder_session::fire_turn_cancel(&handle.turn_cancel)`. The biased
//! `select!` inside the runner's LLM stream loop wins on `await_turn_cancel`,
//! the call returns an empty turn, `run_loop` resets the token and continues;
//! `claim_steers` absorbs the new steer and the next (non-blocking) call
//! replies "recovered". The drain then completes — proving the interrupt
//! worked rather than hanging forever on the first blocking call.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use opencoder_core::Config;
use opencoder_core::ContentBlock;
use opencoder_llm::{ChatRequest, ChatStream, LlmEvent};
use opencoder_store::{Delivery, LibsqlStore, SessionMeta, Store};
use tokio::sync::mpsc;

// ---- BlockingFirstStream: first call hangs, subsequent calls reply ----

/// A `ChatStream` whose first `chat_stream` call never produces an event (the
/// receiver's `recv()` never resolves), forcing `turn_cancel` to win the
/// biased `select!` in the runner. The second and later calls immediately
/// return a `Completed` reply.
struct BlockingFirstStream {
    calls: AtomicUsize,
}

impl BlockingFirstStream {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }
}

impl ChatStream for BlockingFirstStream {
    fn chat_stream(&self, _req: ChatRequest) -> anyhow::Result<mpsc::Receiver<LlmEvent>> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel::<LlmEvent>(128);
        if n == 0 {
            // First call: hold the sender alive forever so `rx.recv()` never
            // resolves; the turn_cancel token must win the biased select.
            tokio::spawn(async move {
                std::future::pending::<()>().await;
                drop(tx);
            });
        } else {
            // Subsequent calls: return a completed reply immediately.
            tokio::spawn(async move {
                let _ = tx
                    .send(LlmEvent::Completed {
                        text: "recovered".into(),
                        tool_calls: vec![],
                        usage: None,
                    })
                    .await;
            });
        }
        Ok(rx)
    }
}

// ---- helpers (adapted from web_drain_contract.rs) ----

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

/// Admit a steer prompt and spawn its drain, returning the admitted seq.
async fn admit_steer(
    state: &opencoder_web::AppState,
    sid: &str,
    prompt: &str,
    client: Arc<dyn ChatStream>,
) -> i64 {
    opencoder_web::handle::admit_and_drain(
        state.handles.clone(),
        state.store.clone(),
        sid,
        prompt.to_string(),
        Vec::new(),
        Delivery::Steer,
        client,
        std::env::temp_dir(),
        Config {
            model: "m/g".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap()
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

// ---- the test ----

#[tokio::test]
async fn steer_mid_drain_fires_turn_cancel_and_recovers() {
    let state = state().await;
    let sid = "steer-cancel-test";
    seed(&state, sid).await;

    let blocking = Arc::new(BlockingFirstStream::new()) as Arc<dyn ChatStream>;

    // First steer prompt: starts the drain, which enters the blocking first
    // chat_stream call (stuck mid-LLM-turn).
    let _seq1 = admit_steer(&state, sid, "first", blocking.clone()).await;

    // Give the drain time to resume/replay and enter the blocking LLM call.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // The drain must be running and stuck in the blocking stream.
    {
        let handles = state.handles.lock().await;
        let h = handles.get(sid).expect("handle exists");
        assert!(
            h.draining.load(Ordering::SeqCst),
            "drain must be running before the second steer"
        );
    }

    // Second steer prompt while the drain is running. This takes the
    // else-branch of `admit_and_drain` and fires `turn_cancel` on the handle,
    // interrupting the in-flight (blocking) LLM turn.
    let _seq2 = admit_steer(&state, sid, "steer-interrupt", blocking.clone()).await;

    // The turn_cancel should have interrupted the blocking call; the loop then
    // absorbs the new steer and the second chat_stream call returns
    // "recovered". Poll for drain completion (bounded — without the interrupt
    // the first call hangs forever, so this guards against a regression that
    // silently drops the turn_cancel).
    let mut completed = false;
    for _ in 0..400 {
        tokio::time::sleep(Duration::from_millis(25)).await;
        let handles = state.handles.lock().await;
        if let Some(h) = handles.get(sid) {
            if !h.draining.load(Ordering::SeqCst) {
                completed = true;
                break;
            }
        }
    }
    assert!(
        completed,
        "drain must complete after steer fires turn_cancel; the blocking first \
         chat_stream call was never interrupted"
    );

    // Behavioral proof the interrupt was a *soft* turn-level cancel (not a hard
    // abort): the runner continued, absorbed the steer, and produced the
    // "recovered" reply from the second (non-blocking) call. Poll the store.
    let mut got_recovered = false;
    for _ in 0..200 {
        if replied(&state, sid, "recovered").await {
            got_recovered = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        got_recovered,
        "expected the 'recovered' reply after the steer was absorbed post-interrupt"
    );

    // The runner must have emitted a `steer_consumed` event (the second steer
    // was absorbed at the next turn boundary) and a terminal `done`.
    let events = state.store.events_after(sid, 0).await.unwrap();
    let kinds: Vec<&str> = events
        .iter()
        .filter_map(|r| r.sse_kind.as_deref())
        .collect();
    assert!(
        kinds.contains(&"steer_consumed"),
        "expected a steer_consumed event after the turn_cancel; got kinds {:?}",
        kinds
    );
    assert!(
        kinds.contains(&"done"),
        "expected a terminal done event; got kinds {:?}",
        kinds
    );
}
