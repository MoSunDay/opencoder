//! Regression tests for two drain-lifecycle bugs in `handle.rs`:
//!
//! - Bug 2 (`drain_to_completion`): the `DrainGuard` (which clears the
//!   `draining` flag) must be dropped only AFTER `flusher.await` completes, so
//!   that the moment `draining` reads `false` the store has already received
//!   every session event. Otherwise a 50 ms-polling reaper can spawn a new
//!   drain while the old flusher is still writing.
//! - Bug 3 (`admit_and_drain`): `fire_child_cancels` must only fire when a
//!   fresh drain is started. Admitting a Queue input to an *already-running*
//!   drain (Branch B) must not hard-cancel that drain's in-flight subagent
//!   children.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use opencoder_core::{Config, ContentBlock, Role};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_store::{Delivery, LibsqlStore, SessionMeta, Store};
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------
// Test helpers (mirrors the local helpers in `web_drain_contract.rs`).
// ---------------------------------------------------------------------------

/// Fresh in-memory AppState backed by an in-memory store.
async fn state() -> Arc<opencoder_web::AppState> {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    Arc::new(opencoder_web::AppState {
        client_override: None,
        store,
        workdir: std::env::temp_dir(),
        handles: opencoder_web::handle::new_handle_map(),
    })
}

/// Seed a session row so the drain can resume it.
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
        })
        .await
        .unwrap();
}

/// Mock that completes a single assistant turn replying `text`.
fn mock_reply(text: &str) -> Arc<dyn ChatStream> {
    Arc::new(
        MockChatClient::new().with_default(vec![LlmEvent::Completed {
            text: text.into(),
            tool_calls: vec![],
            usage: None,
        }]),
    )
}

/// Minimal default config for drain tests (model "m/g").
fn config() -> Config {
    Config {
        model: "m/g".into(),
        ..Default::default()
    }
}

/// Acquire (or create) the handle for `sid` from the shared map.
async fn handle_for(
    state: &opencoder_web::AppState,
    sid: &str,
) -> Arc<opencoder_web::handle::SessionHandle> {
    let mut map = state.handles.lock().await;
    map.entry(sid.to_string())
        .or_insert_with(opencoder_web::handle::SessionHandle::new)
        .clone()
}

/// Poll until the session's drain is idle (`draining` reset).
async fn wait_idle(state: &opencoder_web::AppState, sid: &str) {
    for _ in 0..200 {
        let idle = state
            .handles
            .lock()
            .await
            .get(sid)
            .map(|h| !h.draining.load(Ordering::SeqCst))
            .unwrap_or(true);
        if idle {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("drain for {sid} never went idle");
}

// ---------------------------------------------------------------------------
// Bug 3: Queue admit to a *running* drain must not cancel child subagents.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn queue_admit_to_running_drain_does_not_cancel_children() {
    let st = state().await;
    seed(&st, "s1").await;
    let handle = handle_for(&st, "s1").await;

    // Simulate an already-running drain: `draining` is true, so
    // `started_new_drain` will be false and `fire_child_cancels` must NOT run.
    handle.draining.store(true, Ordering::SeqCst);

    // Register a fake in-flight subagent child cancel token.
    let child_token = CancellationToken::new();
    {
        let mut cancels = handle
            .child_cancels
            .lock()
            .expect("child_cancels mutex poisoned");
        let mut map: HashMap<String, CancellationToken> = HashMap::new();
        map.insert("child-1".to_string(), child_token.clone());
        *cancels = map;
    }

    // Admit a Queue input to the running drain. With the bug, this would
    // unconditionally call `fire_child_cancels` and cancel our child token.
    let seq = opencoder_web::handle::admit_and_drain(
        st.handles.clone(),
        st.store.clone(),
        "s1",
        "follow-up".into(),
        Vec::new(),
        Delivery::Queue,
        mock_reply("ok"),
        std::env::temp_dir(),
        config(),
    )
    .await
    .expect("admit_and_drain must succeed");

    assert!(seq > 0, "input must be admitted");
    // The child's cancel token must NOT have been fired: it belongs to the
    // running drain's turn, and a Queue admit only enqueues for later.
    assert!(
        !child_token.is_cancelled(),
        "Queue delivery to a running drain must not cancel child subagents"
    );
}

// ---------------------------------------------------------------------------
// Steer admit to a *running* drain MUST cancel child subagents — this aligns
// with the TUI's CancelChildrenAndSteer path. Previously only turn_cancel was
// fired and children kept running.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn steer_admit_to_running_drain_cancels_children() {
    let st = state().await;
    seed(&st, "s1").await;
    let handle = handle_for(&st, "s1").await;

    // Simulate an already-running drain: `draining` is true, so
    // `started_new_drain` will be false and the steer-into-running-drain
    // branch fires turn_cancel + fire_child_cancels.
    handle.draining.store(true, Ordering::SeqCst);

    // Register a fake in-flight subagent child cancel token.
    let child_token = CancellationToken::new();
    {
        let mut cancels = handle
            .child_cancels
            .lock()
            .expect("child_cancels mutex poisoned");
        let mut map: HashMap<String, CancellationToken> = HashMap::new();
        map.insert("child-steer".to_string(), child_token.clone());
        *cancels = map;
    }

    // Admit a Steer input to the running drain — this should cancel children.
    let seq = opencoder_web::handle::admit_and_drain(
        st.handles.clone(),
        st.store.clone(),
        "s1",
        "change direction".into(),
        Vec::new(),
        Delivery::Steer,
        mock_reply("ok"),
        std::env::temp_dir(),
        config(),
    )
    .await
    .expect("admit_and_drain must succeed");

    assert!(seq > 0, "input must be admitted");
    assert!(
        child_token.is_cancelled(),
        "Steer delivery to a running drain must cancel child subagents"
    );
}

// ---------------------------------------------------------------------------
// Bug 2: when `draining` clears, all events must already be persisted.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn drain_completion_persists_events_before_clearing_draining() {
    let st = state().await;
    seed(&st, "s1").await;

    // Start a fresh drain (draining was false → started_new_drain = true).
    let _ = opencoder_web::handle::admit_and_drain(
        st.handles.clone(),
        st.store.clone(),
        "s1",
        "hello".into(),
        Vec::new(),
        Delivery::Steer,
        mock_reply("world-reply"),
        std::env::temp_dir(),
        config(),
    )
    .await
    .expect("admit_and_drain must succeed");

    // Wait for the drain to go idle. Because `flusher.await` now runs BEFORE
    // `drop(guard)`, the moment `draining` reads false the store is fully
    // persisted. We check the store immediately — no extra polling.
    wait_idle(&st, "s1").await;

    let msgs = st.store.load_messages("s1").await.expect("load_messages");
    assert!(
        msgs.iter().any(|m| m.role == Role::Assistant),
        "assistant message must be persisted by the time draining clears"
    );
    assert!(
        msgs.iter()
            .flat_map(|m| m.blocks.iter())
            .any(|b| matches!(b, ContentBlock::Text { text } if text.contains("world-reply"))),
        "the assistant's reply text must be persisted by the time draining clears"
    );
}

// ---------------------------------------------------------------------------
// Bug: drain_to_completion must restore cmd_rx BEFORE clearing `draining`.
// The old order (draining=false while cmd_rx still held) let a new drain
// start with cmd_rx.take() == None and silently lose every drain command.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn drain_completion_restores_cmd_rx_before_clearing_draining() {
    let state = state().await;
    let sid = "drain-cmdrx-order".to_string();
    seed(&state, &sid).await;

    // First drain: admit a prompt and wait for the drain to go idle.
    opencoder_web::handle::admit_and_drain(
        state.handles.clone(),
        state.store.clone(),
        &sid,
        "hi".to_string(),
        vec![],
        Delivery::Queue,
        mock_reply("ok"),
        state.workdir.clone(),
        Config::default(),
    )
    .await
    .unwrap();

    let handle = {
        let map = state.handles.lock().await;
        map.get(&sid).expect("handle survives drain").clone()
    };
    for _ in 0..200 {
        if !handle.draining.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(!handle.draining.load(Ordering::SeqCst), "drain must finish");

    // Once idle, cmd_rx must be back in the handle for the next drain.
    {
        let guard = handle.cmd_rx.lock().unwrap();
        assert!(
            guard.is_some(),
            "cmd_rx must be restored after drain goes idle"
        );
    }

    // A second drain runs cleanly end-to-end.
    opencoder_web::handle::admit_and_drain(
        state.handles.clone(),
        state.store.clone(),
        &sid,
        "again".to_string(),
        vec![],
        Delivery::Queue,
        mock_reply("ok2"),
        state.workdir.clone(),
        Config::default(),
    )
    .await
    .unwrap();
    let handle2 = {
        let map = state.handles.lock().await;
        map.get(&sid).expect("handle survives").clone()
    };
    for _ in 0..200 {
        if !handle2.draining.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        !handle2.draining.load(Ordering::SeqCst),
        "second drain must finish"
    );
}
