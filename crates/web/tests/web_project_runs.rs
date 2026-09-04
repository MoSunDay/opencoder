//! `/api/project/*` run lifecycle over the FULL app (signature middleware
//! included): plan/execute 202 + background run on a `MockChatClient`,
//! cancel idempotency, and the uninitialized-service 503 shape. The shared
//! harness lives in `tests/support/project_app.rs`.

mod support;

use std::sync::Arc;

use axum::http::StatusCode;
use opencoder_store::{LibsqlStore, Store};
use serde_json::json;
use support::project_app::{call, done, harness, todo_row, tool_turn, wait_until, TOKEN};

#[tokio::test]
async fn plan_and_execute_run_lifecycle() {
    let h = harness().await;
    let (_, goal) = call(
        &h.app,
        "POST",
        "/api/project/goals",
        Some(json!({ "title": "目标" })),
    )
    .await;
    let gid = goal["id"].as_str().unwrap().to_string();
    let (_, todo) = call(
        &h.app,
        "POST",
        "/api/project/todos",
        Some(json!({ "title": "落地", "draft": "把草稿变成代码" })),
    )
    .await;
    let tid = todo["id"].as_str().unwrap().to_string();

    // v1 plan run: the mocked script answers with the plan markdown.
    h.mock.queue_script(done("# 计划\n1. x"));
    let (status, v) = call(
        &h.app,
        "POST",
        &format!("/api/project/todos/{tid}/plan"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{v}");
    assert!(v["run_id"].as_str().unwrap().starts_with("prun-"), "{v}");

    let planned = wait_until(&h.app, "/api/project/todos", "plan_md lands", |b| {
        b["todos"]
            .as_array()
            .is_some_and(|a| a.iter().any(|t| t["id"] == tid && !t["plan_md"].is_null()))
    })
    .await;
    let row = todo_row(&planned, &tid);
    assert_eq!(row["status"], "planned");
    assert_eq!(row["plan_md"], "# 计划\n1. x");

    // v2 execute run: one bash tool turn, then a completion.
    h.mock.queue_script(tool_turn("先跑一步", "echo ok"));
    h.mock.queue_script(done("完成"));
    let (status, v) = call(
        &h.app,
        "POST",
        &format!("/api/project/todos/{tid}/execute"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{v}");
    wait_until(&h.app, "/api/project/todos", "todo done", |b| {
        b["todos"]
            .as_array()
            .is_some_and(|a| a.iter().any(|t| t["id"] == tid && t["status"] == "done"))
    })
    .await;

    // Run history: newest first — execute v2 over plan v1.
    let (status, runs) = call(
        &h.app,
        "GET",
        &format!("/api/project/todos/{tid}/runs"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{runs}");
    let runs = runs["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0]["kind"], "execute");
    assert_eq!(runs[0]["version"], 2);
    assert_eq!(runs[0]["status"], "done");
    assert_eq!(runs[0]["output_md"], "完成");
    assert_eq!(runs[1]["kind"], "plan");
    assert_eq!(runs[1]["version"], 1);
    assert_eq!(runs[1]["status"], "done");

    // Overview tree carries the goal with its done backlog todo.
    let (status, tree) = call(&h.app, "GET", "/api/project/overview", None).await;
    assert_eq!(status, StatusCode::OK, "{tree}");
    assert_eq!(tree["goals"].as_array().unwrap().len(), 1);
    assert_eq!(tree["goals"][0]["id"], gid.as_str());
    assert_eq!(tree["backlog"].as_array().unwrap().len(), 1);
    assert_eq!(tree["backlog"][0]["status"], "done");

    // Execute without a plan is a 409 conflict.
    let (_, other) = call(
        &h.app,
        "POST",
        "/api/project/todos",
        Some(json!({ "title": "未计划", "draft": "无方案" })),
    )
    .await;
    let oid = other["id"].as_str().unwrap().to_string();
    let (status, v) = call(
        &h.app,
        "POST",
        &format!("/api/project/todos/{oid}/execute"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["ok"], false);
    assert!(v["error"].as_str().unwrap().contains("no plan"));

    // Unknown todo on plan is a 404 with the error body.
    let (status, v) = call(&h.app, "POST", "/api/project/todos/pt-none/plan", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{v}");
    assert_eq!(v["ok"], false);
}

#[tokio::test]
async fn cancel_unknown_run_returns_false_and_503_shape() {
    let h = harness().await;
    // Cancelling an unknown run is idempotent, not an error.
    let (status, v) = call(
        &h.app,
        "POST",
        "/api/project/runs/prun-unknown/cancel",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v, json!({ "cancelled": false }));

    // Router-level sanity on the initialized app.
    let (status, v) = call(&h.app, "GET", "/api/project/goals", None).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["goals"].as_array().unwrap().len(), 0);

    // Uninitialized service (AppState built without init) → 503 everywhere.
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let state = Arc::new(opencoder_web::AppState {
        client_override: None,
        brain: opencoder_web::api_brain::mock_brain(store.clone()),
        store,
        workdir: std::env::temp_dir(),
        handles: opencoder_web::handle::new_handle_map(),
        nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
        controls: Arc::new(opencoder_web::control_state::ControlHub::new()),
        team: opencoder_web::team_state::mock(),
        project: opencoder_web::ProjectService::new(),
    });
    let bare = opencoder_web::build_app(state, Some(TOKEN.into()), false);
    for uri in ["/api/project/goals", "/api/project/overview"] {
        let (status, v) = call(&bare, "GET", uri, None).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{uri}: {v}");
        assert_eq!(v["ok"], false);
        assert!(v["error"].as_str().unwrap().contains("not initialized"));
    }
}
