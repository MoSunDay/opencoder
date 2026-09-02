//! P1: the two historically silent drain-failure paths must surface terminal
//! `error` frames to SSE subscribers (TUI worker contract):
//!   * resume_session failing before the drain even starts (missing session
//!     row) — previously warn-only, leaving the stream hung with no terminal
//!     frame while `draining` reset;
//!   * the run ending in Err without a runner-emitted Error (unit-pinned in
//!     handle_tests via ensure_run_error_frame; the runner's own LLM-failure
//!     emission stays single-count per drain_no_restart_on_error).
//!
//! The resume-failure frame is broadcast-only when the session row is gone
//! (session_events FK requires the row), which is exactly why the live frame
//! is the contract that matters here.

use std::sync::Arc;
use std::time::Duration;

use opencoder_core::Config;
use opencoder_llm::{ChatStream, MockChatClient};
use opencoder_store::LibsqlStore;
use opencoder_web::handle::SessionHandle;

fn dummy_client() -> Arc<dyn ChatStream> {
    Arc::new(MockChatClient::new())
}

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

use opencoder_store::Store;

/// Drain on a session with NO row: the drain must end quickly AND the SSE
/// subscriber must observe a terminal `error` frame instead of hanging.
#[tokio::test]
async fn resume_failure_broadcasts_terminal_error_frame() {
    let state = state().await;
    let sid = "ghost-session";

    let handle = SessionHandle::new();
    let mut rx = handle.tx.subscribe();
    state
        .handles
        .lock()
        .await
        .insert(sid.into(), handle.clone());

    opencoder_web::handle::ensure_drain(
        state.handles.clone(),
        state.store.clone(),
        sid,
        dummy_client(),
        std::env::temp_dir(),
        Config::default(),
    )
    .await;

    // The drain settles (draining resets) rather than spinning forever.
    for _ in 0..500 {
        if !handle.draining.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(
        !handle.draining.load(std::sync::atomic::Ordering::SeqCst),
        "a failed resume must end the drain attempt"
    );

    let evt = rx
        .try_recv()
        .expect("resume failure must broadcast a terminal error frame");
    assert_eq!(evt.kind, "error");
    let msg = evt.data["error"].as_str().expect("error payload string");
    assert!(
        msg.contains("resume failed"),
        "frame must name the resume failure, got: {msg}"
    );
}
