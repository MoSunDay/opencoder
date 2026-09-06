//! Integration tests for the pending-input endpoints (TUI queue-panel parity):
//! list / delete / reorder work against the durable store while a drain is
//! blocked mid-turn (mock `push_hang` holds the LLM call open), plus the
//! delivery-validation error path.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_store::LibsqlStore;
use serde_json::json;
use tokio::sync::Notify;
use tower::ServiceExt;

/// App whose drain mock hangs on the FIRST LLM call (released via `notify`)
/// and answers everything after with a plain Completed — so queued inputs
/// admitted while the drain is stuck stay pending.
async fn hanging_fixture(
    notify: Arc<Notify>,
) -> (axum::Router, Arc<LibsqlStore>, Arc<MockChatClient>) {
    let store = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let workdir = tempfile::tempdir().unwrap().keep();
    std::fs::create_dir_all(workdir.join(".opencoder")).unwrap();
    std::fs::write(
        workdir.join(".opencoder").join("ap.json"),
        r#"{"mode":"off"}"#,
    )
    .unwrap();
    let mock =
        Arc::new(
            MockChatClient::new()
                .push_hang(notify)
                .with_default(vec![LlmEvent::Completed {
                    text: "done".into(),
                    tool_calls: vec![],
                    usage: None,
                }]),
        );
    let state = Arc::new(opencoder_web::AppState {
        brain: opencoder_web::api_brain::mock_brain(store.clone()),
        store: store.clone(),
        workdir,
        handles: opencoder_web::handle::new_handle_map(),
        nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
        controls: Arc::new(opencoder_web::control_state::ControlHub::new()),
        team: opencoder_web::team_state::mock(),
        project: opencoder_web::ProjectService::new(),
        client_override: Some(mock.clone() as Arc<dyn ChatStream>),
    });
    (opencoder_web::build_app(state, None, false), store, mock)
}

async fn hanging_app(notify: Arc<Notify>) -> axum::Router {
    hanging_fixture(notify).await.0
}

async fn create_session(app: &axum::Router) -> String {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&axum::body::to_bytes(resp.into_body(), 4096).await.unwrap())
            .unwrap();
    body["id"].as_str().unwrap().to_string()
}

async fn post_prompt(app: &axum::Router, id: &str, prompt: &str, delivery: &str) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/sessions/{id}/prompt"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"prompt": prompt, "delivery": delivery}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "admit {prompt} failed");
}

async fn get_inputs(app: &axum::Router, id: &str, delivery: &str) -> serde_json::Value {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/sessions/{id}/inputs?delivery={delivery}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), 1 << 20)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(status, StatusCode::OK, "list inputs ({delivery}): {body}");
    body
}

/// 3 queue inputs accumulate while the drain is held mid-turn; delete,
/// reorder, and validation all behave like the TUI queue panel.
#[tokio::test]
async fn queue_inputs_list_delete_and_reorder_while_drain_hangs() {
    let notify = Arc::new(Notify::new());
    let app = hanging_app(notify.clone()).await;
    let id = create_session(&app).await;

    // The steer prompt starts the drain and hangs its first LLM turn; the
    // three queue admissions must all stay pending (promoted_seq NULL).
    post_prompt(&app, &id, "first prompt (steer, hangs the LLM)", "steer").await;
    post_prompt(&app, &id, "queued one", "queue").await;
    post_prompt(&app, &id, "queued two", "queue").await;
    post_prompt(&app, &id, "queued three", "queue").await;

    let mut body = serde_json::Value::Null;
    for _ in 0..100 {
        body = get_inputs(&app, &id, "queue").await;
        if body["inputs"].as_array().is_some_and(|a| a.len() == 3) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let inputs = body["inputs"].as_array().unwrap().clone();
    assert_eq!(inputs.len(), 3, "3 queued inputs must be pending: {body}");
    let prompts: Vec<&str> = inputs
        .iter()
        .map(|i| i["prompt"].as_str().unwrap())
        .collect();
    assert_eq!(prompts, vec!["queued one", "queued two", "queued three"]);
    for i in &inputs {
        assert_eq!(i["delivery"].as_str(), Some("queue"));
        assert!(i["promoted_seq"].is_null(), "must be unpromoted: {i}");
        assert!(i["seq"].as_i64().is_some(), "row seq must be surfaced");
    }

    // DELETE one pending row → 2 remain; deleting a non-pending seq → 404.
    let seq_two = inputs[1]["seq"].as_i64().unwrap();
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/sessions/{id}/inputs/{seq_two}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let after_delete = get_inputs(&app, &id, "queue").await;
    let remaining: Vec<&str> = after_delete["inputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["prompt"].as_str().unwrap())
        .collect();
    assert_eq!(remaining, vec!["queued one", "queued three"]);

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/sessions/{id}/inputs/999999"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "deleting a non-pending seq must 404"
    );

    // REORDER swaps the drain order (listing is by admitted_seq ASC).
    let seq_one = after_delete["inputs"][0]["seq"].as_i64().unwrap();
    let seq_three = after_delete["inputs"][1]["seq"].as_i64().unwrap();
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/sessions/{id}/inputs/reorder"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"a": seq_one, "b": seq_three}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let reordered = get_inputs(&app, &id, "queue").await;
    let order: Vec<&str> = reordered["inputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["prompt"].as_str().unwrap())
        .collect();
    assert_eq!(
        order,
        vec!["queued three", "queued one"],
        "reorder must swap the admitted_seq order"
    );

    // Invalid delivery → 400.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/sessions/{id}/inputs?delivery=bogus"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // Cleanup: release the hang and interrupt the drain so the test runtime
    // can shut down without a stuck task.
    notify.notify_one();
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/sessions/{id}/interrupt"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

/// Defaults and empty sessions: absent delivery lists steer, unknown session
/// yields an empty list (not an error).
#[tokio::test]
async fn list_inputs_defaults_to_steer_and_tolerates_unknown_session() {
    let notify = Arc::new(Notify::new());
    let app = hanging_app(notify).await;
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/sessions/no-such-session/inputs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&axum::body::to_bytes(resp.into_body(), 4096).await.unwrap())
            .unwrap();
    assert_eq!(body["inputs"].as_array().map(Vec::len), Some(0));
}

#[tokio::test]
async fn input_id_retry_returns_original_seq_without_restarting_a_live_turn() {
    let notify = Arc::new(Notify::new());
    let (app, _store, mock) = hanging_fixture(notify.clone()).await;
    let id = create_session(&app).await;
    let post = |prompt: &str| {
        Request::builder()
            .method("POST")
            .uri(format!("/api/sessions/{id}/prompt"))
            .header("content-type", "application/json")
            .body(Body::from(
                json!({"input_id":"initial-execution-1","prompt":prompt}).to_string(),
            ))
            .unwrap()
    };

    let first = app.clone().oneshot(post("hello")).await.unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    let first_body: serde_json::Value =
        serde_json::from_slice(&axum::body::to_bytes(first.into_body(), 4096).await.unwrap())
            .unwrap();
    assert_eq!(first_body["driver_ensured"], true);
    for _ in 0..100 {
        if mock.call_count() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(mock.call_count(), 1, "first turn must be in flight");

    let retry = app.clone().oneshot(post("hello")).await.unwrap();
    assert_eq!(retry.status(), StatusCode::OK);
    let retry_body: serde_json::Value =
        serde_json::from_slice(&axum::body::to_bytes(retry.into_body(), 4096).await.unwrap())
            .unwrap();
    assert_eq!(retry_body["admitted_seq"], first_body["admitted_seq"]);
    assert_eq!(retry_body["driver_ensured"], true);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        mock.call_count(),
        1,
        "retry must not cancel/restart the turn"
    );

    let conflict = app.clone().oneshot(post("different")).await.unwrap();
    assert_eq!(conflict.status(), StatusCode::CONFLICT);

    notify.notify_one();

    let mut consumed_retry = serde_json::Value::Null;
    for _ in 0..100 {
        let response = app.clone().oneshot(post("hello")).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        consumed_retry = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), 4096)
                .await
                .unwrap(),
        )
        .unwrap();
        if consumed_retry["driver_ensured"] == false {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(consumed_retry["driver_ensured"], false);
    assert_eq!(mock.call_count(), 1, "consumed retry must remain inert");
}
