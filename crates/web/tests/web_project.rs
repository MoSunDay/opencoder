//! `/api/project/*` CRUD contract over the FULL app (signature middleware
//! included): goals → milestones → todos with list/read/PATCH semantics.
//! Run-lifecycle cases live in `tests/web_project_runs.rs`; the shared
//! harness lives in `tests/support/project_app.rs`.

mod support;

use axum::http::StatusCode;
use serde_json::{json, Value};
use support::project_app::{call, done, harness, todo_row};

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

/// Executor-dimension write contract (P4): the create/patch bodies carry
/// `executor_kind/executor_ref/executor_spec`, bad kind strings and bad
/// inline specs 400 at the door, double-option null clears, and run rows
/// expose the RESOLVED `executor_kind` (plan runs stay agent).
#[tokio::test]
async fn todo_executor_fields_create_patch_and_run_shape() {
    let h = harness().await;

    // team todo with ref + valid inline spec → round-trips all three.
    let team_spec = serde_json::json!({
        "name": "crew",
        "captain": { "node_id": "act", "name": "队长" },
        "members": [{ "node_id": "explore", "name": "侦察" }]
    })
    .to_string();
    let (status, todo) = call(
        &h.app,
        "POST",
        "/api/project/todos",
        Some(json!({
            "title": "团队活",
            "draft": "多人协作",
            "executor_kind": "team",
            "executor_ref": "  crew-x  ",
            "executor_spec": team_spec,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{todo}");
    let tid = todo["id"].as_str().unwrap().to_string();
    assert_eq!(todo["executor_kind"], "team");
    assert_eq!(todo["executor_ref"], "crew-x", "ref is trimmed");
    assert!(todo["executor_spec"]
        .as_str()
        .unwrap()
        .contains("\"captain\""));
    let (_, list) = call(&h.app, "GET", "/api/project/todos", None).await;
    let row = todo_row(&list, &tid);
    assert_eq!(row["executor_kind"], "team");
    assert_eq!(row["executor_ref"], "crew-x");

    // Unknown kind string → 400 naming it.
    let (status, v) = call(
        &h.app,
        "POST",
        "/api/project/todos",
        Some(json!({ "title": "x", "draft": "y", "executor_kind": "nope" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"]
        .as_str()
        .unwrap()
        .contains("unknown executor_kind: nope"));

    // dag + invalid DagSpec JSON → 400 mentioning the spec problem.
    let (status, v) = call(
        &h.app,
        "POST",
        "/api/project/todos",
        Some(json!({
            "title": "x", "draft": "y", "executor_kind": "dag",
            "executor_spec": "{\"name\":\"d\",\"steps\":[{\"name\":\"s\",\"kind\":{\"type\":\"agent\"}}]}"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"].as_str().unwrap().contains("executor_spec"));

    // agent + spec → 400 (agent takes no spec).
    let (status, v) = call(
        &h.app,
        "POST",
        "/api/project/todos",
        Some(json!({ "title": "x", "draft": "y", "executor_spec": "{}" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"]
        .as_str()
        .unwrap()
        .contains("agent executor takes no spec"));

    // PATCH: null-clear executor_ref (double option) + swap kind; the spec
    // survives the swap only when it validates for the new kind — swapping
    // to dag with a team spec must 400, so clear the spec in the same patch.
    let (status, v) = call(
        &h.app,
        "PATCH",
        &format!("/api/project/todos/{tid}"),
        Some(json!({ "executor_ref": null, "executor_spec": null })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let (status, v) = call(
        &h.app,
        "PATCH",
        &format!("/api/project/todos/{tid}"),
        Some(json!({ "executor_kind": "dag" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let (_, list) = call(&h.app, "GET", "/api/project/todos", None).await;
    let row = todo_row(&list, &tid);
    assert_eq!(row["executor_kind"], "dag");
    assert_eq!(row["executor_ref"], Value::Null, "null cleared the ref");
    assert_eq!(row["executor_spec"], Value::Null, "null cleared the spec");

    // Spec-only patch against the CURRENT kind: dag + a broken spec 400s
    // even without executor_kind in the body.
    let (status, v) = call(
        &h.app,
        "PATCH",
        &format!("/api/project/todos/{tid}"),
        Some(json!({ "executor_spec": "not json" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"].as_str().unwrap().contains("executor_spec"));

    // Run rows carry the resolved executor_kind — a plan run is agent even
    // on a dag todo (planning is executor-agnostic).
    h.mock.queue_script(done("# 计划\n1. x"));
    let (status, v) = call(
        &h.app,
        "POST",
        &format!("/api/project/todos/{tid}/plan"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{v}");
    let planned = support::project_app::wait_until(
        &h.app,
        &format!("/api/project/todos/{tid}/runs"),
        "run lands with executor_kind",
        |b| {
            b["runs"].as_array().is_some_and(|runs| {
                runs.iter()
                    .any(|r| r["executor_kind"] == "agent" && r["kind"] == "plan")
            })
        },
    )
    .await;
    let run = planned["runs"][0].clone();
    assert_eq!(run["executor_kind"], "agent");
    assert_eq!(run["kind"], "plan");
}

#[tokio::test]
async fn patch_kind_only_revalidates_stored_spec() {
    let h = harness().await;

    // team todo with a valid team spec on disk.
    let team_spec = serde_json::json!({
        "name": "crew",
        "captain": { "node_id": "act", "name": "队长" },
        "members": [{ "node_id": "explore", "name": "侦察" }]
    })
    .to_string();
    let (status, todo) = call(
        &h.app,
        "POST",
        "/api/project/todos",
        Some(json!({
            "title": "换型",
            "draft": "存档 spec 复检",
            "executor_kind": "team",
            "executor_spec": team_spec,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{todo}");
    let tid = todo["id"].as_str().unwrap().to_string();

    // kind-only PATCH (spec NOT cleared): the stored team spec must be
    // revalidated against the new dag kind → 400 naming executor_spec.
    let (status, v) = call(
        &h.app,
        "PATCH",
        &format!("/api/project/todos/{tid}"),
        Some(json!({ "executor_kind": "dag" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"].as_str().unwrap().contains("executor_spec"));

    // The rejected patch applied nothing: the todo keeps team + its spec.
    let (_, list) = call(&h.app, "GET", "/api/project/todos", None).await;
    let row = todo_row(&list, &tid);
    assert_eq!(row["executor_kind"], "team");
    assert!(row["executor_spec"]
        .as_str()
        .unwrap()
        .contains("\"captain\""));

    // Happy variant: clear the spec in the same patch → 200.
    let (status, v) = call(
        &h.app,
        "PATCH",
        &format!("/api/project/todos/{tid}"),
        Some(json!({ "executor_kind": "dag", "executor_spec": null })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let (_, list) = call(&h.app, "GET", "/api/project/todos", None).await;
    let row = todo_row(&list, &tid);
    assert_eq!(row["executor_kind"], "dag");
    assert_eq!(row["executor_spec"], Value::Null, "null cleared the spec");
}

#[tokio::test]
async fn standalone_relations_and_protected_deletion() {
    let h = harness().await;
    let (status, milestone) = call(
        &h.app,
        "POST",
        "/api/project/milestones",
        Some(json!({"title":"专项"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{milestone}");
    assert!(milestone["goal_id"].is_null());
    let mid = milestone["id"].as_str().unwrap();
    let (_, todo) = call(
        &h.app,
        "POST",
        "/api/project/todos",
        Some(json!({"title":"任务","draft":"正文","milestone_id":mid})),
    )
    .await;
    let tid = todo["id"].as_str().unwrap();
    let (_, overview) = call(&h.app, "GET", "/api/project/overview", None).await;
    assert_eq!(overview["standalone_milestones"][0]["todos"][0]["id"], tid);
    let (status, _) = call(
        &h.app,
        "DELETE",
        &format!("/api/project/milestones/{mid}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (_, goal) = call(
        &h.app,
        "POST",
        "/api/project/goals",
        Some(json!({"title":"项目"})),
    )
    .await;
    let gid = goal["id"].as_str().unwrap();
    for body in [
        json!({"goal_id":gid}),
        json!({"goal_id":null}),
        json!({"title":"改名"}),
    ] {
        let (status, body) = call(
            &h.app,
            "PATCH",
            &format!("/api/project/milestones/{mid}"),
            Some(body),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    let (_, items) = call(&h.app, "GET", "/api/project/milestones", None).await;
    assert!(items["milestones"][0]["goal_id"].is_null());
    let (status, _) = call(
        &h.app,
        "PATCH",
        &format!("/api/project/todos/{tid}"),
        Some(json!({"milestone_id":null})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(
        &h.app,
        "DELETE",
        &format!("/api/project/milestones/{mid}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, todos) = call(&h.app, "GET", "/api/project/todos", None).await;
    assert_eq!(todos["todos"][0]["draft"], "正文");
}
