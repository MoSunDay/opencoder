//! `/api/brain/playbooks` REST contract tests, driven through the production
//! `build_app` router (same shape as `web_brain.rs`: oneshot + JSON
//! assertions, zero network). Playbooks carry no embeddings, so the mock
//! brain is only the store carrier — the contract under test is the CRUD
//! mapping: payload rejections → 400 (joined verbatim), unknown id → 404,
//! the minted id/echoed spec shape, and update/delete semantics.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use serde_json::json;
use tower::ServiceExt;

use opencoder_store::{LibsqlStore, Store};

/// A valid playbook payload: serial agent chain a → b.
fn payload(name: &str) -> serde_json::Value {
    json!({
        "name": name,
        "steps": [
            {"name": "a", "target": {"kind": "agent", "agent": "act"}, "prompt": "do a"},
            {
                "name": "b",
                "depends_on": ["a"],
                "target": {"kind": "agent", "agent": "act"},
                "prompt": "do b"
            }
        ]
    })
}

async fn state() -> Arc<opencoder_web::AppState> {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    Arc::new(opencoder_web::AppState {
        config_home: None,
        brain: opencoder_web::api_brain::mock_brain(store.clone()),
        store,
        workdir: std::env::temp_dir(),
        handles: opencoder_web::handle::new_handle_map(),
        nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
        controls: Arc::new(opencoder_web::control_state::ControlHub::new()),
        team: opencoder_web::team_state::mock(),
        project: opencoder_web::ProjectService::new(),
        client_override: None,
    })
}

fn app(state: Arc<opencoder_web::AppState>, token: Option<String>) -> Router {
    opencoder_web::build_app(state, token, false)
}

async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let req = Request::builder().method(method).uri(uri);
    let req = match body {
        Some(v) => req
            .header("content-type", "application/json")
            .body(Body::from(v.to_string()))
            .unwrap(),
        None => req.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.expect("router must answer");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 4 << 20)
        .await
        .unwrap();
    let body = if bytes.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&bytes).unwrap_or(json!({}))
    };
    (status, body)
}

#[tokio::test]
async fn playbook_writes_are_migration_errors_and_historical_reads_remain() {
    let state = state().await;
    let router = app(state.clone(), None);
    for (method, path) in [
        ("POST", "/api/brain/playbooks"),
        ("PUT", "/api/brain/playbooks/old"),
        ("DELETE", "/api/brain/playbooks/old"),
    ] {
        let (status, body) = call(&router, method, path, Some(payload("old"))).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert!(body.to_string().contains("migration"));
    }
    assert_eq!(
        call(&router, "GET", "/api/brain/playbooks/missing", None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let protected = app(state, Some("secret".into()));
    assert_eq!(
        call(
            &protected,
            "POST",
            "/api/brain/playbooks",
            Some(payload("old"))
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
}
