//! `/api/project/*` CRUD contract over the FULL app (signature middleware
//! included): goals → milestones → todos with list/read/PATCH semantics.
//! Run-lifecycle cases live in `tests/web_project_runs.rs`; the shared
//! harness lives in `tests/support/project_app.rs`.

mod support;

use axum::http::StatusCode;
use serde_json::{json, Value};
use support::project_app::{call, harness, todo_row};

#[tokio::test]
async fn goal_milestone_todo_crud_contract() {
    let h = harness().await;

    // Goal create → server id, active status, trimmed title.
    let (status, goal) = call(
        &h.app,
        "POST",
        "/api/project/goals",
        Some(json!({ "title": "  目标A  ", "detail_md": "初始" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{goal}");
    let gid = goal["id"].as_str().unwrap().to_string();
    assert!(gid.starts_with("pg-"), "id: {gid}");
    assert_eq!(goal["status"], "active");
    assert_eq!(goal["title"], "目标A");

    // Patch title + detail; list reflects it.
    let (status, v) = call(
        &h.app,
        "PATCH",
        &format!("/api/project/goals/{gid}"),
        Some(json!({ "title": "目标A2", "detail_md": "改后" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let (_, goals) = call(&h.app, "GET", "/api/project/goals", None).await;
    assert_eq!(goals["goals"].as_array().unwrap().len(), 1);
    assert_eq!(goals["goals"][0]["title"], "目标A2");
    assert_eq!(goals["goals"][0]["detail_md"], "改后");

    // Unknown goal_id is a 404 with the shared error body.
    let (status, v) = call(
        &h.app,
        "POST",
        "/api/project/milestones",
        Some(json!({ "goal_id": "pg-bogus", "title": "x" })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{v}");
    assert_eq!(v["ok"], false);
    assert!(v["error"].as_str().unwrap().contains("pg-bogus"));

    // Milestone create + status patch + goal filter.
    let (status, ms) = call(
        &h.app,
        "POST",
        "/api/project/milestones",
        Some(json!({ "goal_id": gid, "title": "里程碑1" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{ms}");
    let mid = ms["id"].as_str().unwrap().to_string();
    assert!(mid.starts_with("pm-"));
    let (status, v) = call(
        &h.app,
        "PATCH",
        &format!("/api/project/milestones/{mid}"),
        Some(json!({ "status": "in_progress" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let (_, list) = call(
        &h.app,
        "GET",
        &format!("/api/project/milestones?goal_id={gid}"),
        None,
    )
    .await;
    assert_eq!(list["milestones"].as_array().unwrap().len(), 1);
    assert_eq!(list["milestones"][0]["status"], "in_progress");
    let (_, unfiltered) = call(&h.app, "GET", "/api/project/milestones", None).await;
    assert_eq!(unfiltered["milestones"].as_array().unwrap().len(), 1);
    let (_, other) = call(
        &h.app,
        "GET",
        "/api/project/milestones?goal_id=pg-none",
        None,
    )
    .await;
    assert_eq!(other["milestones"].as_array().unwrap().len(), 0);

    // Todo under the milestone; JSON null milestone_id clears to backlog.
    let (status, todo) = call(
        &h.app,
        "POST",
        "/api/project/todos",
        Some(json!({ "milestone_id": mid, "title": "待办1", "draft": "草稿" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{todo}");
    let tid = todo["id"].as_str().unwrap().to_string();
    assert!(tid.starts_with("pt-"));
    assert_eq!(todo["status"], "draft");
    assert_eq!(todo["agent"], "act", "default agent");
    let (status, v) = call(
        &h.app,
        "PATCH",
        &format!("/api/project/todos/{tid}"),
        Some(json!({ "milestone_id": null })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let (_, todos) = call(&h.app, "GET", "/api/project/todos", None).await;
    assert_eq!(todo_row(&todos, &tid)["milestone_id"], Value::Null);

    // Missing required field (draft) is an axum Json rejection → 4xx.
    let (status, v) = call(
        &h.app,
        "POST",
        "/api/project/todos",
        Some(json!({ "title": "无草稿" })),
    )
    .await;
    assert!(
        status.is_client_error(),
        "missing required field must 4xx, got {status}: {v}"
    );

    // Unknown ids on patch/delete are 404s.
    let (status, v) = call(
        &h.app,
        "PATCH",
        "/api/project/todos/pt-none",
        Some(json!({ "title": "x" })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{v}");

    // Cascades: todo → milestone → goal empties every list.
    for uri in [
        format!("/api/project/todos/{tid}"),
        format!("/api/project/milestones/{mid}"),
        format!("/api/project/goals/{gid}"),
    ] {
        let (status, v) = call(&h.app, "DELETE", &uri, None).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {v}");
        assert_eq!(v["deleted"], true);
    }
    let (_, todos) = call(&h.app, "GET", "/api/project/todos", None).await;
    assert_eq!(todos["todos"].as_array().unwrap().len(), 0);
    let (_, goals) = call(&h.app, "GET", "/api/project/goals", None).await;
    assert_eq!(goals["goals"].as_array().unwrap().len(), 0);
}
