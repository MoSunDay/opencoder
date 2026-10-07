use super::*;

#[tokio::test]
async fn todo_capability_can_be_saved_before_execution_and_cleared() {
    let h = Harness::new().await;
    let (status, created) = h
        .req(
            Method::POST,
            "/api/project/todos",
            Some(json!({
                "title": "operator task", "draft": "work", "capability_id": "agent-client-code-repair-operator"
            })),
        )
        .await;
    assert_eq!(status, 200, "{created}");
    assert_eq!(
        created["capability_id"],
        "agent-client-code-repair-operator"
    );
    let id = created["id"].as_str().unwrap();
    let path = format!("/api/project/todos/{id}");
    let (_, overview) = h.req(Method::GET, "/api/project/overview", None).await;
    assert_eq!(
        overview["backlog"][0]["capability_id"],
        "agent-client-code-repair-operator"
    );
    let (status, _) = h
        .req(
            Method::PATCH,
            &path,
            Some(json!({"capability_id":"plan-project@2"})),
        )
        .await;
    assert_eq!(status, 200);
    let (_, changed) = h.req(Method::GET, "/api/project/todos", None).await;
    assert_eq!(changed["todos"][0]["capability_id"], "plan-project@2");
    let (status, _) = h
        .req(Method::PATCH, &path, Some(json!({"capability_id":null})))
        .await;
    assert_eq!(status, 200);
    let (_, cleared) = h.req(Method::GET, "/api/project/todos", None).await;
    assert!(cleared["todos"][0]["capability_id"].is_null());
    let (status, _) = h
        .req(
            Method::PATCH,
            &path,
            Some(json!({"capability_id":"invalid\ncapability"})),
        )
        .await;
    assert_eq!(status, 400);
}

#[tokio::test]
async fn goals_initiatives_todos_crud_roundtrip() {
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

    let (status, initiative) = h
        .req(
            Method::POST,
            "/api/project/initiatives",
            Some(json!({"goal_id": goal_id, "title": "M1"})),
        )
        .await;
    assert_eq!(status, 200, "{initiative}");
    let initiative_id = initiative["id"].as_str().unwrap().to_string();

    let (status, todo) = h
        .req(
            Method::POST,
            "/api/project/todos",
            Some(
                json!({"initiative_id": initiative_id, "title": "T2", "draft": "x", "agent": "act"}),
            ),
        )
        .await;
    assert_eq!(status, 200, "{todo}");
    let todo_id = todo["id"].as_str().unwrap().to_string();

    // List filters.
    let (status, body) = h
        .req(
            Method::GET,
            &format!("/api/project/initiatives?goal_id={goal_id}"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["initiatives"].as_array().unwrap().len(), 1);
    let (status, body) = h
        .req(
            Method::GET,
            &format!("/api/project/todos?initiative_id={initiative_id}"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["todos"].as_array().unwrap().len(), 1);

    // Patch + protected initiative deletion paths.
    let (status, body) = h
        .req(
            Method::PATCH,
            &format!("/api/project/todos/{todo_id}"),
            Some(json!({"board_status": "in_progress"})),
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

    // Initiative patch: rename, blank-title guard, unknown id.
    let (status, body) = h
        .req(
            Method::PATCH,
            &format!("/api/project/initiatives/{initiative_id}"),
            Some(json!({"title": "renamed"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"], json!(true));
    let (status, body) = h
        .req(
            Method::PATCH,
            &format!("/api/project/initiatives/{initiative_id}"),
            Some(json!({"title": "  "})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert!(body["error"].as_str().unwrap().contains("empty"), "{body}");
    let (status, body) = h
        .req(
            Method::PATCH,
            "/api/project/initiatives/ms-none",
            Some(json!({"title": "x"})),
        )
        .await;
    assert_eq!(status, 404, "{body}");

    // Nonempty initiatives must first have their TODOs explicitly unlinked.
    let (status, _) = h
        .req(
            Method::POST,
            "/api/project/todos",
            Some(json!({"initiative_id": initiative_id, "title": "T3", "draft": "x"})),
        )
        .await;
    assert_eq!(status, 200);
    let (status, body) = h
        .req(
            Method::DELETE,
            &format!("/api/project/initiatives/{initiative_id}"),
            None,
        )
        .await;
    assert_eq!(status, 409, "{body}");
    let (_, linked) = h
        .req(
            Method::GET,
            &format!("/api/project/todos?initiative_id={initiative_id}"),
            None,
        )
        .await;
    for todo in linked["todos"].as_array().unwrap() {
        let id = todo["id"].as_str().unwrap();
        let (status, _) = h
            .req(
                Method::PATCH,
                &format!("/api/project/todos/{id}"),
                Some(json!({"initiative_id":null})),
            )
            .await;
        assert_eq!(status, 200);
    }
    let (status, body) = h
        .req(
            Method::DELETE,
            &format!("/api/project/initiatives/{initiative_id}"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["deleted"], json!(true));
    let (status, body) = h
        .req(
            Method::GET,
            &format!("/api/project/todos?initiative_id={initiative_id}"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["todos"], json!([]));
    let (status, _) = h
        .req(
            Method::DELETE,
            &format!("/api/project/initiatives/{initiative_id}"),
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

    // Overview lists the saved TODO; execution details are loaded by ID.
    let (status, body) = h.req(Method::GET, "/api/project/overview", None).await;
    assert_eq!(status, 200, "{body}");
    let backlog = body["backlog"].as_array().unwrap();
    let mine = backlog.iter().find(|t| t["id"] == json!(todo_id)).unwrap();
    assert_eq!(mine["board_status"], "backlog");
    assert!(mine.get("execution").is_none(), "{mine}");
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
    h.node.set_project_runs(
        &exec_id,
        json!({"runs": [{"version": 1, "status": "done"}], "more":false}),
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
