use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use opencoder_llm::MockChatClient;
use opencoder_store::{LibsqlStore, Store};
use tower::ServiceExt;

/// Two binary steps with one dependency — the minimal valid workflow.
const SPEC: &str = r#"{"name":"etl-demo","steps":[
    {"name":"fetch","kind":{"type":"binary","resource":"tool"}},
    {"name":"load","depends_on":["fetch"],"kind":{"type":"binary","resource":"tool"}}]}"#;

/// Wrap a raw spec literal in the `DagDefUpsertRequest` envelope.
pub(super) fn spec_body_of(spec: &str) -> String {
    format!(r#"{{"spec":{spec}}}"#)
}

pub(super) fn spec_body() -> String {
    spec_body_of(SPEC)
}

pub(super) struct Ctx {
    pub(super) app: axum::Router,
    pub(super) store: Arc<dyn Store>,
}

pub(super) async fn app() -> Ctx {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let state = Arc::new(opencoder_web::AppState {
        config_home: None,
        brain: opencoder_web::api_brain::mock_brain(store.clone()),
        store: store.clone(),
        workdir: std::env::temp_dir(),
        handles: opencoder_web::handle::new_handle_map(),
        nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
        controls: Arc::new(opencoder_web::control_state::ControlHub::new()),
        team: opencoder_web::team_state::mock(),
        project: opencoder_web::ProjectService::new(),
        client_override: Some(Arc::new(MockChatClient::new())),
    });
    Ctx {
        app: opencoder_web::build_app(state, None, false),
        store,
    }
}

pub(super) async fn send(
    app: &axum::Router,
    req: Request<Body>,
) -> (StatusCode, serde_json::Value) {
    let resp = app.clone().oneshot(req).await.expect("router must answer");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let body = if bytes.is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::json!({}))
    };
    (status, body)
}

pub(super) fn req(method: &str, uri: &str, body: Option<String>) -> Request<Body> {
    match body {
        Some(json) => Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(json)),
        None => Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty()),
    }
    .unwrap()
}

pub(super) async fn register(app: &axum::Router, name: &str) -> String {
    let (_, b) = send(
        app,
        req(
            "POST",
            "/api/nodes/register",
            Some(format!(r#"{{"name":"{name}"}}"#)),
        ),
    )
    .await;
    b["node_id"].as_str().unwrap().into()
}

/// Upsert the sample def; returns its (stable) id.
pub(super) async fn upsert_def(app: &axum::Router) -> String {
    let (s, b) = send(app, req("POST", "/api/dag/defs", Some(spec_body()))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    b["id"].as_str().unwrap().into()
}

pub(super) async fn dispatch(app: &axum::Router, def_id: &str, node_id: Option<&str>) -> String {
    let body = match node_id {
        Some(n) => format!(r#"{{"node_id":"{n}"}}"#),
        None => "{}".to_string(),
    };
    let (s, b) = send(
        app,
        req(
            "POST",
            &format!("/api/dag/defs/{def_id}/dispatch"),
            Some(body),
        ),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    b["run_id"].as_str().unwrap().into()
}

/// Claim for `node_id`; `None` models the 204 idle answer.
pub(super) async fn claim(app: &axum::Router, node_id: &str) -> Option<serde_json::Value> {
    let (s, b) = send(
        app,
        req(
            "GET",
            &format!("/api/nodes/dag/claim?node_id={node_id}"),
            None,
        ),
    )
    .await;
    if s == StatusCode::NO_CONTENT {
        return None;
    }
    assert_eq!(s, StatusCode::OK, "{b}");
    Some(b)
}

/// Upload one event; returns the raw (status, body).
pub(super) async fn upload(
    app: &axum::Router,
    rid: &str,
    events: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    send(
        app,
        req(
            "POST",
            &format!("/api/nodes/dag/runs/{rid}/events"),
            Some(serde_json::json!({ "run_id": rid, "events": events }).to_string()),
        ),
    )
    .await
}
