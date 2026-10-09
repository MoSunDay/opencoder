//! HTTP contract for durable external resource waiting and restart recovery.
use axum::{body::Body, http::Request, routing::post, Json, Router};
use opencoder_core::fleet::*;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use tower::ServiceExt;

async fn call(app: &Router, method: &str, path: &str, value: Value) -> (u16, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("Authorization", "Bearer resource-test-credential")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_vec(&value).unwrap()))
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

async fn freeze(state: &opencoder_control::AppState, id: &str) -> CreateExecution {
    let request = CreateExecution {
        id: id.into(),
        kind: ExecutionKind::Dag,
        target: Some("fixture".into()),
        node_id: Some("offline-node".into()),
        input: json!({"_resource_request":{"units":1}}),
    };
    let fingerprint = opencoder_core::token_hash(&serde_json::to_string(&request).unwrap());
    assert!(state
        .fleet
        .claim_request("execution", id, &fingerprint)
        .await
        .unwrap());
    state
        .fleet
        .prepare_assignment(
            &Assignment {
                private_context: None,
                runtime: None,
                codex: None,
                request: request.clone(),
                definition: None,
                index: ExecutionIndex {
                    id: id.into(),
                    kind: ExecutionKind::Dag,
                    node_id: "offline-node".into(),
                    created_at: 1,
                    status: ExecutionStatus::Pending,
                },
            },
            &fingerprint,
        )
        .await
        .unwrap();
    request
}

#[tokio::test]
async fn waiting_is_durable_and_cancel_never_contacts_the_execution_node() {
    let dir = tempfile::tempdir().unwrap();
    let granted = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = Router::new().route(
        "/resources",
        post({
            let granted = granted.clone();
            let calls = calls.clone();
            move |Json(value): Json<Value>| {
                let granted = granted.clone();
                let calls = calls.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    assert_eq!(value["execution_id"], "dag-waiting");
                    Json(if value["action"] == "inspect" {
                        json!({"allocated":false})
                    } else if granted.load(Ordering::SeqCst) {
                        json!({"status":"granted"})
                    } else {
                        json!({"status":"waiting","reason":"capacity"})
                    })
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/resources", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, provider).await.unwrap() });
    let token = dir.path().join("provider.token");
    std::fs::write(&token, "fixture-provider-credential").unwrap();
    let work = dir.path().join("work");
    let data = dir.path().join("data");
    let state = opencoder_control::new_state(work.clone(), data.clone(), None)
        .await
        .unwrap();
    let request = freeze(&state, "dag-waiting").await;
    let app = opencoder_control::build_app(
        state.clone(),
        Some("resource-test-credential".into()),
        false,
    );
    assert_eq!(
        call(
            &app,
            "PUT",
            "/api/resource-admission-provider",
            json!({"endpoint":endpoint,"token_file":token})
        )
        .await
        .0,
        200
    );
    let (status, index) = call(&app, "POST", "/api/executions", json!(request)).await;
    assert_eq!(status, 202);
    assert_eq!(index["status"], "pending");
    assert_eq!(
        state
            .fleet
            .receipt("execution", "dag-waiting")
            .await
            .unwrap()
            .unwrap()
            .phase,
        "prepared"
    );
    // The API only persists the submission. The Server outbox owns admission.
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    drop(app);
    drop(state);
    let state = opencoder_control::new_state(work, data, None)
        .await
        .unwrap();
    assert_eq!(
        state
            .fleet
            .pending_assignments("", 128)
            .await
            .unwrap()
            .len(),
        1
    );
    let app = opencoder_control::build_app(
        state.clone(),
        Some("resource-test-credential".into()),
        false,
    );
    let (status, index) = call(
        &app,
        "POST",
        "/api/executions/dag-waiting/commands",
        json!({"action":"cancel"}),
    )
    .await;
    assert_eq!(status, 202);
    assert_eq!(index["status"], "cancelled");
    assert!(state
        .fleet
        .pending_assignments("", 128)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        state
            .fleet
            .index("dag-waiting")
            .await
            .unwrap()
            .unwrap()
            .status,
        ExecutionStatus::Cancelled
    );
    server.abort();
}

#[tokio::test]
async fn missing_authority_fails_closed_without_losing_submission() {
    let dir = tempfile::tempdir().unwrap();
    let state =
        opencoder_control::new_state(dir.path().join("work"), dir.path().join("data"), None)
            .await
            .unwrap();
    let request = freeze(&state, "dag-no-provider").await;
    let app = opencoder_control::build_app(
        state.clone(),
        Some("resource-test-credential".into()),
        false,
    );
    let (status, index) = call(&app, "POST", "/api/executions", json!(request)).await;
    assert_eq!(status, 202);
    assert_eq!(index["status"], "pending");
    assert_eq!(
        state
            .fleet
            .pending_assignments("", 128)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        call(
            &app,
            "PUT",
            "/api/resource-admission-provider",
            json!({"endpoint":"file:///tmp/provider","token_file":"relative"})
        )
        .await
        .0,
        400
    );
}
