//! P1: `POST /model` and `POST /agent` must broadcast their switch as a
//! `SessionEvent` (TUI parity: worker.rs emits ModelSwitch/AgentSwitch) AND
//! persist it, so SSE subscribers and replay (`?after=`) both see the switch.
//! Before this the endpoints only wrote store+overrides — reduce.js's
//! `model_switched`/`agent_switched` cases were unreachable dead code on the
//! web surface.

use std::sync::Arc;

use axum::response::IntoResponse;
use opencoder_store::{EventKind, LibsqlStore, SessionMeta, Store};
use opencoder_web::handle::SessionHandle;

async fn state(sid: &str) -> (Arc<opencoder_web::AppState>, Arc<SessionHandle>) {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    store
        .create_session(&SessionMeta {
            id: sid.into(),
            agent: Some("act".into()),
            model: Some("old/model".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    let handle = SessionHandle::new();
    let handles = opencoder_web::handle::new_handle_map();
    handles.lock().await.insert(sid.into(), handle.clone());
    (
        Arc::new(opencoder_web::AppState {
            client_override: None,
            brain: opencoder_web::api_brain::mock_brain(store.clone()),
            store,
            workdir: std::env::temp_dir(),
            handles,
            nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
            controls: Arc::new(opencoder_web::control_state::ControlHub::new()),
            team: opencoder_web::team_state::mock(),
            project: opencoder_web::ProjectService::new(),
        }),
        handle,
    )
}

#[tokio::test]
async fn post_agent_broadcasts_and_persists_agent_switched() {
    let (state, handle) = state("sw-agent").await;
    let mut rx = handle.tx.subscribe();

    let resp = opencoder_web::api::post_agent(
        axum::extract::State(state.clone()),
        axum::extract::Path("sw-agent".to_string()),
        axum::Json(opencoder_web::api::SwitchBody {
            value: "plan".into(),
        }),
    )
    .await
    .into_response();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);

    // Live frame on the session's SSE channel.
    let evt = rx
        .try_recv()
        .expect("agent_switched must be broadcast live");
    assert_eq!(evt.kind, "agent_switched");
    assert_eq!(evt.data["agent"], "plan");

    // ... and persisted exactly once for replay.
    let rows = state.store.events_after("sw-agent", 0).await.unwrap();
    let switches: Vec<_> = rows
        .iter()
        .filter(|r| r.kind == EventKind::AgentSwitched)
        .collect();
    assert_eq!(switches.len(), 1, "exactly one persisted agent_switched");
    assert_eq!(switches[0].payload["agent"], "plan");
    assert_eq!(
        switches[0].sse_kind.as_deref(),
        Some("agent_switched"),
        "replay must restore the granular SSE name"
    );
}

#[tokio::test]
async fn post_model_broadcasts_and_persists_model_switched() {
    let (state, handle) = state("sw-model").await;
    let mut rx = handle.tx.subscribe();

    let resp = opencoder_web::api::post_model(
        axum::extract::State(state.clone()),
        axum::extract::Path("sw-model".to_string()),
        axum::Json(opencoder_web::api::ModelBody {
            value: "new/model".into(),
            persist_default: false,
        }),
    )
    .await
    .into_response();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);

    let evt = rx
        .try_recv()
        .expect("model_switched must be broadcast live");
    assert_eq!(evt.kind, "model_switched");
    assert_eq!(evt.data["model"], "new/model");

    let rows = state.store.events_after("sw-model", 0).await.unwrap();
    let switches: Vec<_> = rows
        .iter()
        .filter(|r| r.kind == EventKind::ModelSwitched)
        .collect();
    assert_eq!(switches.len(), 1, "exactly one persisted model_switched");
    assert_eq!(switches[0].payload["model"], "new/model");
}
