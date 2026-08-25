//! Contract tests for `GET /api/sessions/:id/subagents` (api_subagents.rs):
//! the durable subagent-task listing that backs the SPA's post-refresh card
//! restore and child-transcript drill-down, and `opencode client session
//! tasks`. Assertions: field shape (id/kind/status/child_session_id/prompt/
//! parent_message_id/created_at/updated_at), 200 on an empty list, 404 when
//! the parent session does not exist. Uses the full `build_app` router so
//! route registration is covered, not just the handler.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use opencoder_store::{LibsqlStore, SessionMeta, Store, SubagentStatus, SubagentTaskRecord};

async fn app() -> (axum::Router, Arc<opencoder_web::AppState>) {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let state = Arc::new(opencoder_web::AppState {
        client_override: None,
        store: store.clone(),
        workdir: std::env::temp_dir(),
        handles: opencoder_web::handle::new_handle_map(),
    });
    (opencoder_web::build_app(state.clone(), None, false), state)
}

async fn seed_session(state: &opencoder_web::AppState, sid: &str) {
    state
        .store
        .create_session(&SessionMeta {
            id: sid.to_string(),
            title: None,
            agent: Some("act".into()),
            model: Some("m/g".into()),
            autopilot_mode: None,
            workdir_hash: None,
            created_at: 1_000,
            updated_at: 1_000,
            summary: None,
            summary_seq: None,
            summary_images: vec![],
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: None,
            requirement: None,
            plan_snapshot: None,
            plan_input_count: 0,
        })
        .await
        .unwrap();
}

async fn seed_task(
    state: &opencoder_web::AppState,
    parent: &str,
    child: &str,
    task_id: &str,
    status: SubagentStatus,
) {
    seed_session(state, child).await;
    let rec = SubagentTaskRecord {
        task_id: task_id.to_string(),
        parent_session_id: parent.to_string(),
        child_session_id: child.to_string(),
        parent_message_id: Some(format!("msg_{task_id}")),
        agent: "explore".into(),
        prompt: format!("map {task_id}"),
        result: None,
        status: SubagentStatus::Running, // created Running (see INSERT)
        ok: None,
        started_at: 1_000,
        completed_at: None,
    };
    state.store.create_subagent_task(&rec).await.unwrap();
    if status == SubagentStatus::Completed {
        // result/ok/completed_at are set only by the completion lifecycle
        state
            .store
            .complete_subagent_task(task_id, "done", true)
            .await
            .unwrap();
    }
}

async fn get(app: axum::Router, uri: &str) -> (StatusCode, serde_json::Value) {
    let resp = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    let v = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, v)
}

#[tokio::test]
async fn lists_task_fields_including_child_session() {
    let (router, state) = app().await;
    seed_session(&state, "PARENT").await;
    seed_task(&state, "PARENT", "CHILD1", "t1", SubagentStatus::Completed).await;
    seed_task(&state, "PARENT", "CHILD2", "t2", SubagentStatus::Running).await;

    let (status, v) = get(router, "/api/sessions/PARENT/subagents").await;
    assert_eq!(status, StatusCode::OK);
    let tasks = v["tasks"].as_array().expect("tasks array");
    assert_eq!(tasks.len(), 2, "both seeded tasks listed: {v}");

    let done = tasks.iter().find(|t| t["id"] == "t1").unwrap();
    assert_eq!(done["kind"], "explore", "storage `agent` maps to `kind`");
    assert_eq!(done["status"], "completed", "snake_case status");
    assert_eq!(done["child_session_id"], "CHILD1");
    assert_eq!(done["prompt"], "map t1");
    assert_eq!(done["parent_message_id"], "msg_t1");
    assert_eq!(done["result"], "done");
    assert_eq!(done["ok"], true);
    assert_eq!(done["created_at"], 1_000, "created_at mirrors started_at");
    let updated = done["updated_at"].as_i64().expect("updated_at number");
    assert!(
        updated > 1_000,
        "updated_at mirrors completion time when terminal (got {updated})"
    );

    let running = tasks.iter().find(|t| t["id"] == "t2").unwrap();
    assert_eq!(running["status"], "running");
    assert_eq!(
        running["updated_at"], 1_000,
        "updated_at falls back to started_at while in flight"
    );
}

#[tokio::test]
async fn empty_list_is_200_missing_session_is_404() {
    let (router, state) = app().await;
    seed_session(&state, "EMPTY").await;

    let (status, v) = get(router, "/api/sessions/EMPTY/subagents").await;
    assert_eq!(status, StatusCode::OK, "present-but-empty is a normal 200");
    assert_eq!(
        v["tasks"].as_array().map(Vec::len),
        Some(0),
        "empty session lists zero tasks"
    );

    let (router2, _) = app().await;
    let (status, v) = get(router2, "/api/sessions/NOPE/subagents").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(v["ok"], false, "error body shape: {v}");
}
