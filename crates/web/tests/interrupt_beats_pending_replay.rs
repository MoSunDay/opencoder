//! Integration test: POST /interrupt wins over the steer/queue watcher's
//! defense-in-depth replay. When a queue input is admitted mid-drain, a
//! watcher task polls until the drain exits and would normally restart the
//! drain to consume the still-pending row. But if the drain ended because the
//! user hit interrupt (or the session was deleted), the watcher must NOT
//! resurrect the run — the pending row stays durably admitted and is consumed
//! by the next user-initiated drain instead.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, State};
use opencoder_core::Config;
use opencoder_llm::{ChatRequest, ChatStream, LlmEvent};
use opencoder_store::{Delivery, LibsqlStore, SessionMeta, Store};
use tokio::sync::mpsc;

/// A `ChatStream` whose every call hangs forever (receiver never resolves):
/// the drain enters the first LLM turn and stays there until a cancel token
/// wins the biased select in the runner.
struct HangingStream {
    calls: AtomicUsize,
}

impl ChatStream for HangingStream {
    fn chat_stream(&self, _req: ChatRequest) -> anyhow::Result<mpsc::Receiver<LlmEvent>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel::<LlmEvent>(128);
        tokio::spawn(async move {
            std::future::pending::<()>().await;
            drop(tx);
        });
        Ok(rx)
    }
}

async fn state() -> Arc<opencoder_web::AppState> {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    Arc::new(opencoder_web::AppState {
        client_override: None,
        store,
        workdir: std::env::temp_dir(),
        handles: opencoder_web::handle::new_handle_map(),
    })
}

async fn seed(state: &opencoder_web::AppState, sid: &str) {
    state
        .store
        .create_session(&SessionMeta {
            id: sid.to_string(),
            title: None,
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
            plan_snapshot: None,
            plan_input_count: 0,
        })
        .await
        .unwrap();
}

async fn admit(
    state: &opencoder_web::AppState,
    sid: &str,
    prompt: &str,
    delivery: Delivery,
    client: Arc<dyn ChatStream>,
) {
    opencoder_web::handle::admit_and_drain(
        state.handles.clone(),
        state.store.clone(),
        sid,
        prompt.to_string(),
        Vec::new(),
        delivery,
        client,
        std::env::temp_dir(),
        Config {
            model: "m/g".into(),
            ..Config::default()
        },
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn interrupt_cancels_drain_watcher_does_not_resurrect() {
    let state = state().await;
    let sid = "interrupt-wins";
    seed(&state, sid).await;

    let hanging = Arc::new(HangingStream {
        calls: AtomicUsize::new(0),
    });

    // First prompt: starts the drain, which hangs inside the first LLM turn.
    admit(&state, sid, "first", Delivery::Steer, hanging.clone()).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    {
        let handles = state.handles.lock().await;
        assert!(
            handles.get(sid).unwrap().draining.load(Ordering::SeqCst),
            "drain must be running (stuck in the hanging LLM turn)"
        );
    }

    // Queue input admitted mid-drain: takes the else-branch (no turn cancel —
    // queue inputs are consumed at idle) and spawns the drain watcher.
    admit(
        &state,
        sid,
        "queued follow-up",
        Delivery::Queue,
        hanging.clone(),
    )
    .await;
    let pending_before = state
        .store
        .pending_inputs(sid, Delivery::Queue)
        .await
        .unwrap()
        .len();
    assert_eq!(pending_before, 1, "queue row admitted and still pending");

    // The user interrupts via the real endpoint: the drain breaks out of the
    // hanging LLM turn and exits with the queue row STILL pending.
    opencoder_web::api::post_interrupt(State(state.clone()), Path(sid.to_string())).await;

    // The drain exits; the watcher polls draining=false, sees the pending
    // queue row, and must refuse to restart because the cancel token fired.
    // (The old behavior re-armed a fresh token and re-ran the LLM.)
    tokio::time::sleep(Duration::from_millis(800)).await;
    {
        let handles = state.handles.lock().await;
        let h = handles.get(sid).expect("handle retained by subscribers");
        assert!(
            !h.draining.load(Ordering::SeqCst),
            "interrupted drain must stay down — watcher must not resurrect it"
        );
    }

    // No additional LLM call was made after the interrupt.
    assert_eq!(
        hanging.calls.load(Ordering::SeqCst),
        1,
        "exactly one LLM call — no resurrection of the cancelled run"
    );

    // The queue row is still durably pending, waiting for the next
    // user-initiated drain.
    let pending_after = state
        .store
        .pending_inputs(sid, Delivery::Queue)
        .await
        .unwrap()
        .len();
    assert_eq!(pending_after, 1, "queue row stays pending after interrupt");
}
