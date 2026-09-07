//! Project surface: goal→milestone→todo CRUD, overview aggregation, the
//! plan/act node affinity lifecycle and run views.

use opencoder_core::fleet::{ExecutionKind, ExecutionStatus};
use reqwest::Method;
use serde_json::json;

use crate::support::Harness;

async fn seed_todo(h: &Harness) -> String {
    seed_todo_with_kind(h, None).await
}

/// Same seed but with an explicit executor_kind (e.g. brain pre-resolution).
async fn seed_todo_with_kind(h: &Harness, executor_kind: Option<&str>) -> String {
    let mut body = json!({"title": "T1", "draft": "do it"});
    if let Some(kind) = executor_kind {
        body["executor_kind"] = json!(kind);
    }
    let (status, body) = h.req(Method::POST, "/api/project/todos", Some(body)).await;
    assert_eq!(status, 200, "{body}");
    body["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn goals_milestones_todos_crud_roundtrip() {
    let h = Harness::new().await;
    let (status, goal) = h
        .req(
            Method::POST,
            "/api/project/goals",
            Some(json!({"title": "G1", "detail_md": "d"})),
        )
        .await;
    assert_eq!(status, 200, "{goal}");
    let goal_id = goal["id"].as_str().unwrap().to_string();

    let (status, _) = h
        .req(
            Method::POST,
            "/api/project/goals",
            Some(json!({"title": "  "})),
        )
        .await;
    assert_eq!(status, 400);

    let (status, milestone) = h
        .req(
            Method::POST,
            "/api/project/milestones",
            Some(json!({"goal_id": goal_id, "title": "M1"})),
        )
        .await;
    assert_eq!(status, 200, "{milestone}");
    let milestone_id = milestone["id"].as_str().unwrap().to_string();

    let (status, todo) = h
        .req(
            Method::POST,
            "/api/project/todos",
            Some(
                json!({"milestone_id": milestone_id, "title": "T2", "draft": "x", "agent": "act"}),
            ),
        )
        .await;
    assert_eq!(status, 200, "{todo}");
    let todo_id = todo["id"].as_str().unwrap().to_string();

    // List filters.
    let (status, body) = h
        .req(
            Method::GET,
            &format!("/api/project/milestones?goal_id={goal_id}"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["milestones"].as_array().unwrap().len(), 1);
    let (status, body) = h
        .req(
            Method::GET,
            &format!("/api/project/todos?milestone_id={milestone_id}"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["todos"].as_array().unwrap().len(), 1);

    // Patch + delete cascade paths.
    let (status, body) = h
        .req(
            Method::PATCH,
            &format!("/api/project/todos/{todo_id}"),
            Some(json!({"status": "in_progress"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"], json!(true));
    let (status, body) = h
        .req(
            Method::DELETE,
            &format!("/api/project/todos/{todo_id}"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["deleted"], json!(true));
    let (status, _) = h
        .req(
            Method::DELETE,
            &format!("/api/project/todos/{todo_id}"),
            None,
        )
        .await;
    assert_eq!(status, 404);

    // Milestone patch: rename, blank-title guard, unknown id.
    let (status, body) = h
        .req(
            Method::PATCH,
            &format!("/api/project/milestones/{milestone_id}"),
            Some(json!({"title": "renamed"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"], json!(true));
    let (status, body) = h
        .req(
            Method::PATCH,
            &format!("/api/project/milestones/{milestone_id}"),
            Some(json!({"title": "  "})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert!(body["error"].as_str().unwrap().contains("empty"), "{body}");
    let (status, body) = h
        .req(
            Method::PATCH,
            "/api/project/milestones/ms-none",
            Some(json!({"title": "x"})),
        )
        .await;
    assert_eq!(status, 404, "{body}");

    // Milestone delete cascades its todos (deleted, not re-parented).
    let (status, _) = h
        .req(
            Method::POST,
            "/api/project/todos",
            Some(json!({"milestone_id": milestone_id, "title": "T3", "draft": "x"})),
        )
        .await;
    assert_eq!(status, 200);
    let (status, body) = h
        .req(
            Method::DELETE,
            &format!("/api/project/milestones/{milestone_id}"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["deleted"], json!(true));
    let (status, body) = h
        .req(
            Method::GET,
            &format!("/api/project/todos?milestone_id={milestone_id}"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["todos"], json!([]));
    let (status, _) = h
        .req(
            Method::DELETE,
            &format!("/api/project/milestones/{milestone_id}"),
            None,
        )
        .await;
    assert_eq!(status, 404);

    let (status, body) = h
        .req(
            Method::PATCH,
            &format!("/api/project/goals/{goal_id}"),
            Some(json!({"status": "archived"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = h
        .req(
            Method::DELETE,
            &format!("/api/project/goals/{goal_id}"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");
}

#[tokio::test]
async fn overview_aggregates_backlog_and_plan_act_lifecycle() {
    let h = Harness::new().await;
    let todo_id = seed_todo(&h).await;

    // Plan on a fresh todo creates the project-<todo> execution (202).
    let (status, body) = h
        .req(
            Method::POST,
            &format!("/api/project/todos/{todo_id}/plan"),
            Some(json!({})),
        )
        .await;
    assert_eq!(status, 202, "{body}");
    assert_eq!(body["id"], json!(format!("project-{todo_id}")));
    assert_eq!(body["kind"], json!("project"));
    assert_eq!(body["node_id"], json!("node-e2e"));

    // Second plan routes a `plan` command to the owning node with the
    // server-resolved snapshot injected.
    h.node.set_command(
        &format!("project-{todo_id}"),
        "plan",
        200,
        json!({"id": format!("project-{todo_id}"), "status": "planning"}),
    );
    let (status, body) = h
        .req(
            Method::POST,
            &format!("/api/project/todos/{todo_id}/plan"),
            Some(json!({})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["status"], json!("planning"));
    let seen = h.node.seen_commands();
    let plan_cmd = seen
        .iter()
        .find(|(id, action, _)| id == &format!("project-{todo_id}") && action == "plan")
        .expect("plan command forwarded");
    assert!(
        plan_cmd.2["snapshot"].is_object(),
        "snapshot injected: {plan_cmd:?}"
    );

    // Execute follows the same affinity.
    h.node.set_command(
        &format!("project-{todo_id}"),
        "execute",
        200,
        json!({"id": format!("project-{todo_id}"), "status": "running"}),
    );
    let (status, body) = h
        .req(
            Method::POST,
            &format!("/api/project/todos/{todo_id}/execute"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["status"], json!("running"));

    // Plan on an unknown todo: resolution fails before any node call.
    let (status, body) = h
        .req(Method::POST, "/api/project/todos/todo-none/plan", None)
        .await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["error"], json!("todo not found"));

    // Overview lists the backlog todo with its execution view merged.
    let (status, body) = h.req(Method::GET, "/api/project/overview", None).await;
    assert_eq!(status, 200, "{body}");
    let backlog = body["backlog"].as_array().unwrap();
    let mine = backlog.iter().find(|t| t["id"] == json!(todo_id)).unwrap();
    assert!(mine["execution"].is_object(), "{mine}");
}

#[tokio::test]
async fn run_views_and_cancel_follow_node_ownership() {
    let h = Harness::new().await;
    let todo_id = seed_todo(&h).await;
    // No execution yet → empty runs, no node traffic.
    let (status, body) = h
        .req(
            Method::GET,
            &format!("/api/project/todos/{todo_id}/runs"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["runs"], json!([]));

    let (status, _) = h
        .req(
            Method::POST,
            &format!("/api/project/todos/{todo_id}/plan"),
            None,
        )
        .await;
    assert_eq!(status, 202);
    let exec_id = format!("project-{todo_id}");
    h.node.set_inspect(
        &exec_id,
        json!({"execution": {"id": exec_id, "status": "running"},
               "runs": [{"version": 1, "status": "done"}]}),
    );
    let (status, body) = h
        .req(
            Method::GET,
            &format!("/api/project/todos/{todo_id}/runs"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["runs"][0]["version"], json!(1));

    h.node.set_command(
        &exec_id,
        "cancel",
        200,
        json!({"id": exec_id, "status": "cancelled"}),
    );
    let (status, body) = h
        .req(
            Method::POST,
            &format!("/api/project/runs/{exec_id}/cancel"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["status"], json!("cancelled"));
}

#[tokio::test]
async fn overview_nests_goals_milestones_and_backlog() {
    let h = Harness::new().await;
    let (status, goal) = h
        .req(
            Method::POST,
            "/api/project/goals",
            Some(json!({"title": "G"})),
        )
        .await;
    assert_eq!(status, 200, "{goal}");
    let goal_id = goal["id"].as_str().unwrap().to_string();
    let (status, milestone) = h
        .req(
            Method::POST,
            "/api/project/milestones",
            Some(json!({"goal_id": goal_id, "title": "M"})),
        )
        .await;
    assert_eq!(status, 200, "{milestone}");
    let milestone_id = milestone["id"].as_str().unwrap().to_string();
    let (status, todo) = h
        .req(
            Method::POST,
            "/api/project/todos",
            Some(json!({"milestone_id": milestone_id, "title": "T", "draft": "d"})),
        )
        .await;
    assert_eq!(status, 200, "{todo}");
    let nested_id = todo["id"].as_str().unwrap().to_string();
    let backlog_id = seed_todo(&h).await;

    let (status, body) = h.req(Method::GET, "/api/project/overview", None).await;
    assert_eq!(status, 200, "{body}");
    let goals = body["goals"].as_array().unwrap();
    assert_eq!(goals.len(), 1);
    assert_eq!(goals[0]["id"], json!(goal_id));
    let milestones = goals[0]["milestones"].as_array().unwrap();
    assert_eq!(milestones.len(), 1);
    assert_eq!(milestones[0]["id"], json!(milestone_id));
    let nested = milestones[0]["todos"].as_array().unwrap();
    assert_eq!(nested.len(), 1);
    assert_eq!(nested[0]["id"], json!(nested_id));
    assert_eq!(nested[0]["milestone_id"], json!(milestone_id));

    let backlog = body["backlog"].as_array().unwrap();
    assert_eq!(backlog.len(), 1);
    assert_eq!(backlog[0]["id"], json!(backlog_id));
    assert!(backlog[0]["milestone_id"].is_null(), "{backlog:?}");
}

#[tokio::test]
async fn overview_merges_live_todo_state_from_node_inspect() {
    let h = Harness::new().await;
    let todo_id = seed_todo(&h).await;
    let (status, _) = h
        .req(
            Method::POST,
            &format!("/api/project/todos/{todo_id}/plan"),
            Some(json!({})),
        )
        .await;
    assert_eq!(status, 202);
    let exec_id = format!("project-{todo_id}");
    h.node.set_inspect(
        &exec_id,
        json!({"todo": {"status": "running", "plan_md": "# plan", "active_session_id": "s-7"}}),
    );

    let (status, body) = h.req(Method::GET, "/api/project/overview", None).await;
    assert_eq!(status, 200, "{body}");
    let mine = body["backlog"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == json!(todo_id))
        .unwrap();
    assert!(mine["execution"].is_object(), "{mine}");
    assert_eq!(mine["execution"]["id"], json!(exec_id));
    assert_eq!(mine["status"], json!("running"));
    assert_eq!(mine["plan_md"], json!("# plan"));
    assert_eq!(mine["active_session_id"], json!("s-7"));
}

#[tokio::test]
async fn overview_reports_detail_error_when_inspect_fails() {
    let h = Harness::new().await;
    let todo_id = seed_todo(&h).await;
    // Execution present in the index but the node holds no detail reply.
    let exec_id = format!("project-{todo_id}");
    h.put_index(&exec_id, ExecutionKind::Project, ExecutionStatus::Idle)
        .await;

    let (status, body) = h.req(Method::GET, "/api/project/overview", None).await;
    assert_eq!(status, 200, "{body}");
    let mine = body["backlog"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == json!(todo_id))
        .unwrap();
    assert!(mine["execution"].is_object(), "{mine}");
    assert_eq!(mine["detail_error"]["error"], json!("execution not found"));
}

#[tokio::test]
async fn todo_runs_propagate_node_inspect_failure() {
    let h = Harness::new().await;
    let todo_id = seed_todo(&h).await;
    let exec_id = format!("project-{todo_id}");
    h.put_index(&exec_id, ExecutionKind::Project, ExecutionStatus::Idle)
        .await;

    let (status, body) = h
        .req(
            Method::GET,
            &format!("/api/project/todos/{todo_id}/runs"),
            None,
        )
        .await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["error"], json!("execution not found"));
}

#[tokio::test]
async fn plan_pins_node_and_forwards_node_errors() {
    let h = Harness::new().await;
    let todo_id = seed_todo(&h).await;
    let exec_id = format!("project-{todo_id}");
    // Fresh submit honors a node_id pin from the request body.
    let (status, body) = h
        .req(
            Method::POST,
            &format!("/api/project/todos/{todo_id}/plan"),
            Some(json!({"node_id": "node-e2e"})),
        )
        .await;
    assert_eq!(status, 202, "{body}");
    assert_eq!(body["id"], json!(exec_id));
    assert_eq!(body["node_id"], json!("node-e2e"));

    // A replayed plan forwards the node's non-2xx reply verbatim.
    h.node
        .set_command(&exec_id, "plan", 409, json!({"error": "already planning"}));
    let (status, body) = h
        .req(
            Method::POST,
            &format!("/api/project/todos/{todo_id}/plan"),
            Some(json!({})),
        )
        .await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["error"], json!("already planning"));
}

#[tokio::test]
async fn plan_after_todo_delete_resolves_404() {
    let h = Harness::new().await;
    let todo_id = seed_todo(&h).await;
    let (status, _) = h
        .req(
            Method::POST,
            &format!("/api/project/todos/{todo_id}/plan"),
            Some(json!({})),
        )
        .await;
    assert_eq!(status, 202);
    let (status, body) = h
        .req(
            Method::DELETE,
            &format!("/api/project/todos/{todo_id}"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");

    // The execution still exists in the index; the snapshot resolve fails.
    let (status, body) = h
        .req(
            Method::POST,
            &format!("/api/project/todos/{todo_id}/plan"),
            Some(json!({})),
        )
        .await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["error"], json!("todo not found"));
}

#[tokio::test]
async fn execute_on_fresh_todo_takes_the_submit_branch() {
    let h = Harness::new().await;
    let todo_id = seed_todo(&h).await;
    let (status, body) = h
        .req(
            Method::POST,
            &format!("/api/project/todos/{todo_id}/execute"),
            None,
        )
        .await;
    assert_eq!(status, 202, "{body}");
    assert_eq!(body["id"], json!(format!("project-{todo_id}")));
    assert_eq!(body["kind"], json!("project"));
    assert_eq!(body["node_id"], json!("node-e2e"));
}

#[tokio::test]
async fn cancel_unknown_project_run_is_404() {
    let h = Harness::new().await;
    let (status, body) = h
        .req(
            Method::POST,
            "/api/project/runs/project-unknown/cancel",
            None,
        )
        .await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["error"], json!("execution id not found"));
}

#[tokio::test]
async fn brain_todo_execute_preresolves_empty_library_to_default_agent() {
    let h = Harness::new().await;
    let todo_id = seed_todo_with_kind(&h, Some("brain")).await;

    // Plan first (202) so the project-<todo> execution exists and execute
    // takes the node-affinity command branch.
    let (status, body) = h
        .req(
            Method::POST,
            &format!("/api/project/todos/{todo_id}/plan"),
            Some(json!({})),
        )
        .await;
    assert_eq!(status, 202, "{body}");
    let exec_id = format!("project-{todo_id}");

    h.node.set_command(
        &exec_id,
        "execute",
        200,
        json!({"id": exec_id, "status": "running"}),
    );
    let (status, body) = h
        .req(
            Method::POST,
            &format!("/api/project/todos/{todo_id}/execute"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");

    // The harness store starts with an EMPTY capability library: execute
    // pre-resolves to the default agent override (same first-use default
    // as the brain dispatch surface) instead of a 400.
    let seen = h.node.seen_commands();
    let execute_cmd = seen
        .iter()
        .find(|(id, action, _)| id == &exec_id && action == "execute")
        .expect("execute command forwarded");
    assert_eq!(
        execute_cmd.2["brain"],
        json!({"kind": "agent", "ref": "act", "capability_id": null, "plan_id": null}),
        "input.brain carries the default-agent resolution: {execute_cmd:?}"
    );
}
