//! DELETE /api/nodes/:id/dialogs (compat route) guard rails: an unknown node
//! answers 404, and a registered node that is not connected reports the hub's
//! offline error instead of touching any data. The full happy path (node
//! delegation + index deletion) is covered by the e2e suite with a scripted
//! node.
use axum::{body::Body, http::Request};
use opencoder_core::fleet::NodeRegistration;
use serde_json::{json, Value};
use tower::ServiceExt;

async fn call(app: &axum::Router, method: &str, path: &str) -> (u16, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("Authorization", "Bearer dialogs-clear-test")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn delete_dialogs_unknown_node_answers_404() {
    let dir = tempfile::tempdir().unwrap();
    let state = opencoder_control::new_state(
        dir.path().join("work"),
        dir.path().join("data"),
        None,
    )
    .await
    .unwrap();
    let app = opencoder_control::build_app(state.clone(), Some("dialogs-clear-test".into()), false);
    let (status, body) = call(&app, "DELETE", "/api/nodes/no-such-node/dialogs").await;
    assert_eq!(status, 404);
    assert_eq!(body["error"], json!("node not found"));
}

#[tokio::test]
async fn delete_dialogs_offline_node_reports_hub_error() {
    let dir = tempfile::tempdir().unwrap();
    let state = opencoder_control::new_state(
        dir.path().join("work"),
        dir.path().join("data"),
        None,
    )
    .await
    .unwrap();
    state
        .fleet
        .register(&NodeRegistration {
            id: "node-offline".into(),
            name: "node-offline".into(),
            version: "test".into(),
            protocol_version: opencoder_core::fleet::PROTOCOL_VERSION,
            maintenance_agent_id: "act".into(),
            kinds: vec![opencoder_core::fleet::ExecutionKind::Operator],
        })
        .await
        .unwrap();
    let app = opencoder_control::build_app(state.clone(), Some("dialogs-clear-test".into()), false);
    let (status, body) = call(&app, "DELETE", "/api/nodes/node-offline/dialogs").await;
    assert_eq!(status, 503, "{body}");
    assert_eq!(body["error"], json!("node offline"));
}
