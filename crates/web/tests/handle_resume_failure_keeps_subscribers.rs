//! Regression tests: a drain whose `resume_session` fails (e.g. the session
//! row is missing) must NOT unconditionally remove the session handle from the
//! handles map. Live SSE subscribers still hold that handle's broadcast
//! receiver; removing the map entry orphans them (a later prompt creates a NEW
//! handle/tx they never receive, and their `release_events_subscriber` would
//! decrement the fresh instance's counter — underflow). The handle may only be
//! reclaimed when zero subscribers are attached; otherwise the normal eviction
//! path (last subscriber leaves while idle) reclaims it later.
//!
//! Drain failure is triggered via `handle::ensure_drain` for a session id that
//! has NO row in the store: unlike `admit_and_drain` it does not admit a prompt
//! first, so the spawned `drain_to_completion` fails at `resume_session` with
//! "session not found" and hits the resume-failure branch under test.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use opencoder_core::{Config, SseEvt};
use opencoder_llm::MockChatClient;
use opencoder_store::{LibsqlStore, Store};
use tokio::sync::broadcast;

/// Fresh in-memory AppState (tests call the drain seam directly, no router).
async fn state() -> Arc<opencoder_web::AppState> {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    Arc::new(opencoder_web::AppState {
        client_override: None,
        brain: opencoder_web::api_brain::mock_brain(store.clone()),
        store,
        workdir: std::env::temp_dir(),
        handles: opencoder_web::handle::new_handle_map(),
        nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
        controls: Arc::new(opencoder_web::control_state::ControlHub::new()),
        team: opencoder_web::team_state::mock(),
        project: opencoder_web::ProjectService::new(),
    })
}

/// Minimal default config for drain tests (model "m/g").
fn config() -> Config {
    Config {
        model: "m/g".into(),
        ..Default::default()
    }
}

/// Trigger a drain that is guaranteed to fail at `resume_session`: the session
/// id has no row in the store, and `ensure_drain` admits no prompt first.
async fn trigger_failing_drain(state: &opencoder_web::AppState, sid: &str) {
    opencoder_web::handle::ensure_drain(
        state.handles.clone(),
        state.store.clone(),
        sid,
        Arc::new(MockChatClient::new()) as Arc<dyn opencoder_llm::ChatStream>,
        std::env::temp_dir(),
        config(),
    )
    .await;
}

/// Poll until the spawned drain for `sid` has finished (draining reset or the
/// handle gone) — the resume-failure branch runs inside that task.
async fn wait_drain_settled(state: &opencoder_web::AppState, sid: &str) {
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
    panic!("drain for {sid} never settled");
}

/// Poll until the handle for `sid` is absent from the map (drain reclaimed it).
async fn wait_handle_gone(state: &opencoder_web::AppState, sid: &str) {
    for _ in 0..200 {
        if !state.handles.lock().await.contains_key(sid) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("handle for {sid} was never reclaimed");
}

/// Attach an SSE subscriber exactly the way `GET /events` does: under the map
/// lock, get-or-create the handle, bump `subscribers`, take a broadcast
/// receiver. Returns (handle, receiver).
async fn subscribe(
    state: &opencoder_web::AppState,
    sid: &str,
) -> (
    Arc<opencoder_web::handle::SessionHandle>,
    broadcast::Receiver<SseEvt>,
) {
    let mut map = state.handles.lock().await;
    let handle = map
        .entry(sid.to_string())
        .or_insert_with(opencoder_web::handle::SessionHandle::new)
        .clone();
    handle.subscribers.fetch_add(1, Ordering::SeqCst);
    (handle.clone(), handle.tx.subscribe())
}

/// With a live tracked subscriber (counter > 0) attached, a failed resume must
/// KEEP the handle in the map — same instance, still broadcasting to the
/// subscriber's receiver.
#[tokio::test]
async fn resume_failure_keeps_handle_with_live_subscribers() {
    let st = state().await;
    let sid = "ghost-tracked";
    let (handle, mut rx) = subscribe(&st, sid).await;
    assert_eq!(handle.subscribers.load(Ordering::SeqCst), 1);

    trigger_failing_drain(&st, sid).await;
    wait_drain_settled(&st, sid).await;

    let map = st.handles.lock().await;
    let kept = map
        .get(sid)
        .unwrap_or_else(|| panic!("handle must survive a failed resume while subscribers live"));
    assert!(
        Arc::ptr_eq(kept, &handle),
        "the kept entry must be the SAME handle instance the subscriber attached to"
    );
    assert!(!kept.draining.load(Ordering::SeqCst));

    // The subscriber is still attached: an event broadcast on the live handle
    // must arrive on its receiver (a NEW handle/tx would leave it stranded).
    drop(map);

    // First, the resume-failure terminal frame itself must have reached this
    // subscriber (P1: failed resumes broadcast an `error` instead of dying
    // silently — this is what un-hangs a connected SSE stream).
    let first = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("subscriber must receive the resume-failure frame")
        .expect("receiver must not be closed");
    assert_eq!(first.kind, "error");
    assert!(
        first.data["error"]
            .as_str()
            .unwrap_or("")
            .contains("resume failed"),
        "frame must name the resume failure, got: {first:?}"
    );

    // Then a later broadcast on the kept handle still arrives: the receiver
    // was never orphaned.
    let sent = SseEvt {
        kind: "done".into(),
        data: serde_json::json!({"still":"attached"}),
        ts: 1,
        seq: None,
    };
    handle.tx.send(sent.clone()).expect("broadcast send");
    let got = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("subscriber must still receive events on the kept handle")
        .expect("receiver must not be closed");
    assert_eq!(got.kind, sent.kind);
    assert_eq!(got.data, sent.data);
}

/// With ZERO subscribers, a failed resume must reclaim the handle (the map
/// cannot grow without bound on ids that never resume).
#[tokio::test]
async fn resume_failure_removes_handle_with_zero_subscribers() {
    let st = state().await;
    let sid = "ghost-untracked";

    // ensure_drain creates the handle itself; nobody subscribes.
    assert!(st.handles.lock().await.get(sid).is_none());
    trigger_failing_drain(&st, sid).await;
    wait_handle_gone(&st, sid).await;
}

/// Defense-in-depth: even if the tracked counter missed a subscriber (counter
/// 0 but a broadcast receiver is still held), the failed drain must keep the
/// handle — `tx.receiver_count()` is checked alongside the counter.
#[tokio::test]
async fn resume_failure_keeps_handle_with_uncounted_receiver() {
    let st = state().await;
    let sid = "ghost-uncounted";

    // Subscribe WITHOUT bumping the counter (counter-missed receiver).
    let (handle, _rx) = {
        let mut map = st.handles.lock().await;
        let handle = map
            .entry(sid.to_string())
            .or_insert_with(opencoder_web::handle::SessionHandle::new)
            .clone();
        (handle.clone(), handle.tx.subscribe())
    };
    assert_eq!(handle.subscribers.load(Ordering::SeqCst), 0);
    assert_eq!(handle.tx.receiver_count(), 1);

    trigger_failing_drain(&st, sid).await;
    wait_drain_settled(&st, sid).await;

    let map = st.handles.lock().await;
    let kept = map
        .get(sid)
        .unwrap_or_else(|| panic!("handle must survive while a receiver is still held"));
    assert!(Arc::ptr_eq(kept, &handle));
}
