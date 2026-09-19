//! `/api/brain/plans` + `/api/brain/dispatch` REST contract tests, driven
//! through the production `build_app` router. The brain runtime is backed by
//! a SHARED `MockChatClient` (script queued from the test, embed = pure
//! hash), so the planner LLM call is fully scripted: identical texts embed
//! identically (cosine 1.0) and a threshold of 0.98 makes branch routing
//! deterministic.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use serde_json::json;
use tower::ServiceExt;

use opencoder_llm::{ChatStream, MockChatClient};
use opencoder_store::{LibsqlStore, Store};

async fn state() -> (Arc<opencoder_web::AppState>, Arc<MockChatClient>) {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mock = Arc::new(MockChatClient::new());
    let client: Arc<dyn ChatStream> = mock.clone();
    let brain = opencoder_brain::Runtime::new(store.clone(), client, "mock-embed")
        .with_chat_model("planner-chat");
    (
        Arc::new(opencoder_web::AppState {
            brain,
            store,
            workdir: std::env::temp_dir(),
            handles: opencoder_web::handle::new_handle_map(),
            nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
            controls: Arc::new(opencoder_web::control_state::ControlHub::new()),
            team: opencoder_web::team_state::mock(),
            project: opencoder_web::ProjectService::new(),
            client_override: None,
        }),
        mock,
    )
}

fn app(state: Arc<opencoder_web::AppState>) -> Router {
    opencoder_web::build_app(state, None, false)
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
async fn legacy_planners_reject_writes_and_preserve_historical_queries() {
    let (state, mock) = state().await;
    let record=opencoder_store::BrainPlanRecord{id:"old".into(),situation:"old".into(),situation_digest:"old".into(),chat_model:"old".into(),tree_json:r#"{"threshold":0.5,"root":{"id":"leaf","kind":"leaf","capability_id":"old","reason":"old"}}"#.into(),created_at:1};
    state.store.save_brain_plan(&record).await.unwrap();
    let app = app(state.clone());
    for path in ["/api/brain/plans", "/api/brain/dispatch"] {
        let (status, body) = call(&app, "POST", path, Some(json!({"situation":"work"}))).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert!(body.to_string().contains("migration"));
    }
    let (status, body) = call(&app, "GET", "/api/brain/plans/old", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["plan"]["tree_json"], record.tree_json);
    assert_eq!(mock.call_count(), 0);
}
