//! Contract tests for `DELETE /api/sessions?keep=:id` (api_subagents.rs) —
//! the web parity of the TUI `/task` clear-all:
//! - keeps the target session and FK-cascades the rest (messages/events/
//!   subagent tasks go with their sessions, including subagent children);
//! - 409 when any live handle is draining (gate_clear_all semantics);
//! - evicts non-keep live handles with the delete_session teardown while
//!   keeping the target's handle;
//! - 404 for a missing keep id, 400 without the keep param, idempotent 200
//!   with `removed: 0` when nothing is left to clear.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use opencoder_store::{LibsqlStore, SessionMeta, Store, SubagentStatus, SubagentTaskRecord};

async fn app() -> (axum::Router, Arc<opencoder_web::AppState>) {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let state = Arc::new(opencoder_web::AppState {
        client_override: None,
        brain: opencoder_web::api_brain::mock_brain(store.clone()),
        store: store.clone(),
        workdir: std::env::temp_dir(),
        handles: opencoder_web::handle::new_handle_map(),
        nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
        controls: Arc::new(opencoder_web::control_state::ControlHub::new()),
        team: opencoder_web::team_state::mock(),
        project: opencoder_web::ProjectService::new(),
    });
    (opencoder_web::build_app(state.clone(), None, false), state)
}

async fn seed_session(state: &opencoder_web::AppState, sid: &str) {
    state
        .store
        .create_session(&SessionMeta {
            id: sid.to_string(),
            title: None,
            agent: Some("act".into()),
            model: None,
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

async fn clear(app: axum::Router, uri: &str) -> (StatusCode, serde_json::Value) {
    let resp = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let v = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, v)
}

#[tokio::test]
async fn keeps_target_and_cascades_the_rest() {
    let (router, state) = app().await;
    seed_session(&state, "KEEP").await;
    seed_session(&state, "GONE").await;
    seed_session(&state, "CHILD").await; // subagent child of GONE
    state
        .store
        .create_subagent_task(&SubagentTaskRecord {
            task_id: "t1".into(),
            parent_session_id: "GONE".into(),
            child_session_id: "CHILD".into(),
            parent_message_id: None,
            agent: "build".into(),
            prompt: "p".into(),
            result: None,
            status: SubagentStatus::Running,
            ok: None,
            started_at: 0,
            completed_at: None,
        })
        .await
        .unwrap();

    let (status, v) = clear(router, "/api/sessions?keep=KEEP").await;
    assert_eq!(status, StatusCode::OK, "body: {v}");
    assert_eq!(v["removed"], 2, "GONE + CHILD removed, KEEP stays");

    assert!(state.store.get_session("KEEP").await.unwrap().is_some());
    assert!(state.store.get_session("GONE").await.unwrap().is_none());
    assert!(
        state.store.get_session("CHILD").await.unwrap().is_none(),
        "subagent child sessions are cleared with the rest"
    );
    assert!(
        state.store.get_subagent_task("t1").await.unwrap().is_none(),
        "subagent task rows cascade with their parent session"
    );

    // Idempotent: nothing left to clear.
    let (router2, state2) = app().await;
    seed_session(&state2, "ONLY").await;
    let (status, v) = clear(router2, "/api/sessions?keep=ONLY").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["removed"], 0);
}

#[tokio::test]
async fn refused_with_409_while_any_handle_draining() {
    let (router, state) = app().await;
    seed_session(&state, "KEEP").await;
    seed_session(&state, "BUSY").await;
    let h = opencoder_web::handle::SessionHandle::new();
    h.draining.store(true, std::sync::atomic::Ordering::SeqCst);
    state.handles.lock().await.insert("BUSY".into(), h);

    let (status, v) = clear(router, "/api/sessions?keep=KEEP").await;
    assert_eq!(status, StatusCode::CONFLICT, "draining gate: {v}");
    assert!(
        state.store.get_session("BUSY").await.unwrap().is_some(),
        "409 must not delete anything"
    );
    assert!(state.store.get_session("KEEP").await.unwrap().is_some());

    // The KEEP session draining also refuses: its running subagent's child
    // session is still being written to (TUI gate_clear_all semantics).
    let (router2, state2) = app().await;
    seed_session(&state2, "KEEP").await;
    let hk = opencoder_web::handle::SessionHandle::new();
    hk.draining.store(true, std::sync::atomic::Ordering::SeqCst);
    state2.handles.lock().await.insert("KEEP".into(), hk);
    let (status, _) = clear(router2, "/api/sessions?keep=KEEP").await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn evicts_non_keep_handles_and_keeps_the_targets() {
    let (router, state) = app().await;
    seed_session(&state, "KEEP").await;
    seed_session(&state, "IDLE").await;
    state
        .handles
        .lock()
        .await
        .insert("KEEP".into(), opencoder_web::handle::SessionHandle::new());
    state
        .handles
        .lock()
        .await
        .insert("IDLE".into(), opencoder_web::handle::SessionHandle::new());

    let (status, _) = clear(router, "/api/sessions?keep=KEEP").await;
    assert_eq!(status, StatusCode::OK);
    let map = state.handles.lock().await;
    assert!(map.contains_key("KEEP"), "kept session keeps its handle");
    assert!(
        !map.contains_key("IDLE"),
        "cleared session's handle evicted"
    );
}

#[tokio::test]
async fn missing_keep_is_404_and_absent_keep_param_is_400() {
    let (router, _) = app().await;
    let (status, v) = clear(router, "/api/sessions?keep=NOPE").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {v}");

    let (router2, _) = app().await;
    let (status, v) = clear(router2, "/api/sessions").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {v}");
    assert_eq!(v["ok"], false);
}
