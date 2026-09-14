//! Registration deletion leaves execution indexes and node-owned data intact.
use std::sync::Arc;

use axum::{body::Body, http::Request};
use opencoder_core::{
    fleet::*,
    identity::{Identity, Role},
};
use serde_json::{json, Value};
use tower::ServiceExt;

fn registration(id: &str) -> NodeRegistration {
    NodeRegistration {
        id: id.into(),
        name: id.into(),
        version: "test".into(),
        protocol_version: PROTOCOL_VERSION,
        maintenance_agent_id: "act".into(),
        kinds: vec![ExecutionKind::Agent],
    }
}

async fn call(app: &axum::Router, method: &str, path: &str) -> (u16, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("Authorization", "Bearer registration-test")
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
async fn delete_registration_keeps_indexes_and_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("work");
    let data = dir.path().join("data");
    let state = opencoder_control::new_state(work.clone(), data.clone(), None)
        .await
        .unwrap();
    for id in ["node-a", "node-b"] {
        state.fleet.register(&registration(id)).await.unwrap();
    }
    let record = ExecutionIndex {
        id: "agent-history".into(),
        created_at: 7,
        kind: ExecutionKind::Agent,
        node_id: "node-a".into(),
        status: ExecutionStatus::Running,
    };
    state.fleet.put_index(&record).await.unwrap();
    drop(state);

    // Load the catalog from disk just as a restarted server does.
    let state = opencoder_control::new_state(work.clone(), data.clone(), None)
        .await
        .unwrap();
    let app = opencoder_control::build_app(state.clone(), Some("registration-test".into()), false);
    assert_eq!(
        call(&app, "GET", "/api/nodes").await.1["nodes"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        call(&app, "DELETE", "/api/nodes/node-a").await,
        (200, json!({"ok":true}))
    );
    let (status, body) = call(&app, "GET", "/api/nodes").await;
    assert_eq!(status, 200);
    assert_eq!(body["nodes"].as_array().unwrap().len(), 1);
    assert_eq!(body["nodes"][0]["id"], "node-b");
    let (_, history) = call(&app, "GET", "/api/executions").await;
    assert_eq!(history["executions"], json!([record]));
    assert_eq!(call(&app, "DELETE", "/api/nodes/node-a").await.0, 404);
    drop(app);
    drop(state);

    let state = opencoder_control::new_state(work, data, None)
        .await
        .unwrap();
    assert_eq!(state.hub.views().await.len(), 1);
    assert_eq!(
        serde_json::to_value(state.fleet.index(&record.id).await.unwrap().unwrap()).unwrap(),
        json!(record)
    );
    // A later explicit re-registration can reuse the node id and its history.
    state.fleet.register(&registration("node-a")).await.unwrap();
    assert_eq!(state.fleet.nodes().await.unwrap().len(), 2);
    assert_eq!(
        state
            .fleet
            .indexes(Some("node-a"), None, 10)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn node_registration_delete_requires_admin_identity() {
    let dir = tempfile::tempdir().unwrap();
    let mut state =
        opencoder_control::new_state(dir.path().join("work"), dir.path().join("data"), None)
            .await
            .unwrap();
    state.fleet.register(&registration("node-a")).await.unwrap();
    Arc::get_mut(&mut state).unwrap().hub =
        Arc::new(opencoder_control::transport::Hub::new(vec![registration(
            "node-a",
        )]));
    let app = opencoder_control::build_app(state.clone(), Some("registration-test".into()), false);
    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/nodes/node-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 401);
    // No auth middleware: inject the already-authenticated non-admin identity
    // to exercise the role gate without minting credentials.
    for role in [Role::User, Role::Root] {
        let app = opencoder_control::build_app(state.clone(), None, false);
        let mut request = Request::builder()
            .method("DELETE")
            .uri("/api/nodes/node-a")
            .body(Body::empty())
            .unwrap();
        request.extensions_mut().insert(Identity {
            name: "user".into(),
            role,
        });
        assert_eq!(app.oneshot(request).await.unwrap().status().as_u16(), 403);
    }
    assert_eq!(state.fleet.nodes().await.unwrap().len(), 1);
}
