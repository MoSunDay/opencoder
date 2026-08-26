//! Node-task cancellation flow (`POST /api/nodes/:node_id/tasks/:tid/cancel`).
//!
//! A pending cancel collapses immediately — its closure event streams as
//! `done` with `payload.cancel = true`; a running cancel answers `202
//! cancelling` and reaches the worker through the heartbeat's
//! `cancel_task_ids`. Harness mirrors `nodes_ops.rs`.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use futures::StreamExt;
use opencoder_llm::MockChatClient;
use opencoder_store::LibsqlStore;
use tower::ServiceExt;

async fn app() -> axum::Router {
    let state = Arc::new(opencoder_web::AppState {
        store: Arc::new(LibsqlStore::open_memory().await.unwrap()),
        workdir: std::env::temp_dir(),
        handles: opencoder_web::handle::new_handle_map(),
        nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
        client_override: Some(Arc::new(MockChatClient::new())),
    });
    opencoder_web::build_app(state, None, false)
}

async fn send(app: &axum::Router, req: Request<Body>) -> (StatusCode, serde_json::Value) {
    let resp = app.clone().oneshot(req).await.expect("router answers");
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

fn req(method: &str, uri: &str, body: Option<String>) -> Request<Body> {
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

async fn register(app: &axum::Router, name: &str) -> String {
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

/// Dispatch one task; returns (task_id, session_id).
async fn dispatch(app: &axum::Router, node_id: &str, prompt: &str) -> (String, String) {
    let (_, d) = send(
        app,
        req(
            "POST",
            &format!("/api/nodes/{node_id}/tasks"),
            Some(format!(r#"{{"prompt":"{prompt}"}}"#)),
        ),
    )
    .await;
    (
        d["task_id"].as_str().unwrap().into(),
        d["session_id"].as_str().unwrap().into(),
    )
}

async fn claim(app: &axum::Router, node_id: &str) {
    let (s, c) = send(
        app,
        req(
            "GET",
            &format!("/api/nodes/tasks/claim?node_id={node_id}"),
            None,
        ),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{c}");
}

async fn task_status(app: &axum::Router, node_id: &str) -> String {
    let (_, t) = send(
        app,
        req("GET", &format!("/api/nodes/{node_id}/tasks"), None),
    )
    .await;
    t["tasks"][0]["status"].as_str().unwrap_or("").into()
}

/// Read SSE wire text until `until` appears.
async fn read_sse_text(resp: axum::response::Response, until: &str) -> String {
    let mut stream = resp.into_body().into_data_stream();
    let mut text = String::new();
    for _ in 0..40 {
        match tokio::time::timeout(Duration::from_millis(200), stream.next()).await {
            Ok(Some(Ok(bytes))) => {
                text.push_str(&String::from_utf8_lossy(&bytes));
                if text.contains(until) {
                    break;
                }
            }
            _ => break,
        }
    }
    text
}

/// Last SSE frame as (event-name, data-json).
async fn last_frame(resp: axum::response::Response) -> (String, serde_json::Value) {
    // NOTE: `done` only appears once per run in these tests, so it doubles as
    // the end-of-stream marker for the read loop.
    let text = read_sse_text(resp, "\"task_id\"").await;
    let blocks: Vec<&str> = text
        .split("\n\n")
        .filter(|b| b.contains("event:"))
        .collect();
    let block = blocks.last().expect("at least one SSE frame");
    let name = block
        .lines()
        .find_map(|l| l.strip_prefix("event: "))
        .unwrap_or_default()
        .to_string();
    let data = block
        .lines()
        .find_map(|l| l.strip_prefix("data: "))
        .unwrap_or("{}");
    (name, serde_json::from_str(data).unwrap())
}

#[tokio::test]
async fn cancelling_pending_task_completes_immediately_and_streams_done() {
    let app = app().await;
    let node = register(&app, "cp").await;
    let (tid, _) = dispatch(&app, &node, "never runs").await;

    // Pending cancel collapses right away.
    let (cs, cb) = send(
        &app,
        req(
            "POST",
            &format!("/api/nodes/{node}/tasks/{tid}/cancel"),
            None,
        ),
    )
    .await;
    assert_eq!(cs, StatusCode::OK, "{cb}");
    assert_eq!(cb["phase"], "cancelled");
    assert_eq!(task_status(&app, &node).await, "cancelled");

    // The closure event streams as `done` with payload.cancel = true.
    let resp = app
        .clone()
        .oneshot(req("GET", &format!("/api/nodes/tasks/{tid}/events"), None))
        .await
        .unwrap();
    let (name, data) = last_frame(resp).await;
    assert_eq!(name, "done");
    assert_eq!(data["cancel"], serde_json::json!(true));
    assert_eq!(data["ok"], serde_json::json!(true));
    assert_eq!(data["task_id"], tid.as_str());

    // A terminal task is no longer cancellable.
    let (again, ab) = send(
        &app,
        req(
            "POST",
            &format!("/api/nodes/{node}/tasks/{tid}/cancel"),
            None,
        ),
    )
    .await;
    assert_eq!(again, StatusCode::CONFLICT, "{ab}");
}

#[tokio::test]
async fn cancelling_running_task_answers_202_then_travels_via_heartbeat() {
    let app = app().await;
    let node = register(&app, "cr").await;
    let (tid, _) = dispatch(&app, &node, "long job").await;
    claim(&app, &node).await; // pending -> running

    let (cs, cb) = send(
        &app,
        req(
            "POST",
            &format!("/api/nodes/{node}/tasks/{tid}/cancel"),
            None,
        ),
    )
    .await;
    assert_eq!(cs, StatusCode::ACCEPTED, "{cb}");
    assert_eq!(cb["phase"], "cancelling");
    assert_eq!(task_status(&app, &node).await, "cancelling");

    // The worker picks the instruction up on its next heartbeat.
    let (_, hb) = send(
        &app,
        req(
            "POST",
            &format!("/api/nodes/{node}/heartbeat"),
            Some("{}".into()),
        ),
    )
    .await;
    assert_eq!(
        hb["cancel_task_ids"],
        serde_json::json!([tid]),
        "heartbeat must carry the cancelling task id"
    );
}
