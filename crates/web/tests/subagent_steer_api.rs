//! Functional tests for the `post_subagent_steer` HTTP handler.
//!
//! These exercise the steer endpoint directly (no network) against a real
//! libsql store, asserting the behavioral contracts:
//! - `steer_running_subagent_returns_ok`: a Running task admits a steer to the
//!   child session's pending queue and returns the `admitted_seq`.
//! - `steer_{completed,failed,cancelled}_subagent_returns_409`: a non-running
//!   task is rejected with 409 Conflict (no input admitted).
//! - `steer_nonexistent_task_returns_404`: a missing task_id is rejected 404.
//!
//! The `app()` builder + store seeding mirror the patterns in `web_contract.rs`,
//! restricted to the single subagent-steer route under test.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::post;
use axum::Router;
use tower::ServiceExt;
use uuid::Uuid;

use opencoder_store::{Delivery, LibsqlStore, SessionMeta, Store, SubagentStatus, SubagentTaskRecord};

/// Build a thin test router exposing only the subagent steer route over a
/// shared in-memory store (mirrors the `app()` pattern in web_contract.rs).
async fn app() -> (Router, Arc<opencoder_web::AppState>) {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let workdir = std::env::temp_dir();
    let state = Arc::new(opencoder_web::AppState {
        client_override: None,
        store: store.clone(),
        workdir,
        handles: opencoder_web::handle::new_handle_map(),
    });
    let app = Router::new()
        .route(
            "/api/sessions/:id/subagents/:task_id/steer",
            post(opencoder_web::api::post_subagent_steer),
        )
        .with_state(state.clone());
    (app, state)
}

/// Seed a session row with a minimal valid meta.
async fn seed_session(state: &opencoder_web::AppState, sid: &str) {
    state
        .store
        .create_session(&SessionMeta {
            id: sid.to_string(),
            title: None,
            agent: Some("act".into()),
            model: Some("m/g".into()),
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
        })
        .await
        .unwrap();
}

/// Seed parent + child sessions and a subagent task with the given status.
async fn seed_subagent(
    state: &opencoder_web::AppState,
    parent_sid: &str,
    child_sid: &str,
    task_id: &str,
    status: SubagentStatus,
) {
    seed_session(state, parent_sid).await;
    seed_session(state, child_sid).await;
    let rec = SubagentTaskRecord {
        task_id: task_id.to_string(),
        parent_session_id: parent_sid.to_string(),
        child_session_id: child_sid.to_string(),
        parent_message_id: None,
        agent: "explore".into(),
        prompt: "test".into(),
        result: None,
        status,
        ok: None,
        started_at: 0,
        completed_at: None,
    };
    state.store.create_subagent_task(&rec).await.unwrap();
}

/// POST a steer request and return the response. Centralized so every test
/// shares the exact same request shape.
async fn post_steer(app: Router, parent_sid: &str, task_id: &str) -> axum::response::Response {
    let body = serde_json::json!({"prompt": "redirect here", "images": []});
    app.oneshot(
        Request::builder()
            .method("POST")
            .uri(format!(
                "/api/sessions/{parent_sid}/subagents/{task_id}/steer"
            ))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn steer_running_subagent_returns_ok() {
    let (app, state) = app().await;
    let parent_sid = Uuid::new_v4().to_string();
    let child_sid = Uuid::new_v4().to_string();
    let task_id = Uuid::new_v4().to_string();
    seed_subagent(
        &state,
        &parent_sid,
        &child_sid,
        &task_id,
        SubagentStatus::Running,
    )
    .await;

    let resp = post_steer(app, &parent_sid, &task_id).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["ok"], true);
    let seq = v["admitted_seq"].as_i64().unwrap();
    assert!(seq > 0, "admitted_seq should be positive, got {seq}");

    // The steer must land on the *child* session's pending steer queue.
    let pending = state
        .store
        .pending_inputs(&child_sid, Delivery::Steer)
        .await
        .unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].prompt, "redirect here");
    assert_eq!(pending[0].admitted_seq, seq);
}

#[tokio::test]
async fn steer_completed_subagent_returns_409() {
    let (app, state) = app().await;
    let parent_sid = Uuid::new_v4().to_string();
    let child_sid = Uuid::new_v4().to_string();
    let task_id = Uuid::new_v4().to_string();
    seed_subagent(
        &state,
        &parent_sid,
        &child_sid,
        &task_id,
        SubagentStatus::Completed,
    )
    .await;

    let resp = post_steer(app, &parent_sid, &task_id).await;
    assert_eq!(resp.status(), StatusCode::CONFLICT);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["ok"], false);

    // Nothing should have been admitted to the child.
    let pending = state
        .store
        .pending_inputs(&child_sid, Delivery::Steer)
        .await
        .unwrap();
    assert!(pending.is_empty(), "no steer should be admitted for a completed task");
}

#[tokio::test]
async fn steer_failed_subagent_returns_409() {
    let (app, state) = app().await;
    let parent_sid = Uuid::new_v4().to_string();
    let child_sid = Uuid::new_v4().to_string();
    let task_id = Uuid::new_v4().to_string();
    seed_subagent(
        &state,
        &parent_sid,
        &child_sid,
        &task_id,
        SubagentStatus::Failed,
    )
    .await;

    let resp = post_steer(app, &parent_sid, &task_id).await;
    assert_eq!(resp.status(), StatusCode::CONFLICT);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["ok"], false);

    let pending = state
        .store
        .pending_inputs(&child_sid, Delivery::Steer)
        .await
        .unwrap();
    assert!(pending.is_empty(), "no steer should be admitted for a failed task");
}

#[tokio::test]
async fn steer_cancelled_subagent_returns_409() {
    let (app, state) = app().await;
    let parent_sid = Uuid::new_v4().to_string();
    let child_sid = Uuid::new_v4().to_string();
    let task_id = Uuid::new_v4().to_string();
    seed_subagent(
        &state,
        &parent_sid,
        &child_sid,
        &task_id,
        SubagentStatus::Cancelled,
    )
    .await;

    let resp = post_steer(app, &parent_sid, &task_id).await;
    assert_eq!(resp.status(), StatusCode::CONFLICT);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["ok"], false);

    let pending = state
        .store
        .pending_inputs(&child_sid, Delivery::Steer)
        .await
        .unwrap();
    assert!(
        pending.is_empty(),
        "no steer should be admitted for a cancelled task"
    );
}

#[tokio::test]
async fn steer_nonexistent_task_returns_404() {
    let (app, state) = app().await;
    let parent_sid = Uuid::new_v4().to_string();
    seed_session(&state, &parent_sid).await;
    // A task_id that was never created.
    let task_id = Uuid::new_v4().to_string();

    let resp = post_steer(app, &parent_sid, &task_id).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["ok"], false);
}
