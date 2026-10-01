use super::*;

#[tokio::test]
async fn overview_nests_goals_initiatives_and_backlog() {
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
    let (status, initiative) = h
        .req(
            Method::POST,
            "/api/project/initiatives",
            Some(json!({"goal_id": goal_id, "title": "M"})),
        )
        .await;
    assert_eq!(status, 200, "{initiative}");
    let initiative_id = initiative["id"].as_str().unwrap().to_string();
    let (status, todo) = h
        .req(
            Method::POST,
            "/api/project/todos",
            Some(json!({"initiative_id": initiative_id, "title": "T", "draft": "d"})),
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
    let initiatives = goals[0]["initiatives"].as_array().unwrap();
    assert_eq!(initiatives.len(), 1);
    assert_eq!(initiatives[0]["id"], json!(initiative_id));
    let nested = initiatives[0]["todos"].as_array().unwrap();
    assert_eq!(nested.len(), 1);
    assert_eq!(nested[0]["id"], json!(nested_id));
    assert_eq!(nested[0]["initiative_id"], json!(initiative_id));

    let backlog = body["backlog"].as_array().unwrap();
    assert_eq!(backlog.len(), 1);
    assert_eq!(backlog[0]["id"], json!(backlog_id));
    assert!(backlog[0]["initiative_id"].is_null(), "{backlog:?}");
}

#[tokio::test]
async fn overview_keeps_saved_todo_state_independent_of_node_inspect() {
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
    assert!(mine.get("execution").is_none(), "{mine}");
    assert_eq!(mine["status"], json!("draft"));
    assert_eq!(mine["board_status"], json!("backlog"));
    assert!(mine["plan_md"].is_null());
    assert!(mine["active_session_id"].is_null());
}

#[tokio::test]
async fn overview_keeps_saved_todo_when_the_node_lost_the_execution() {
    let h = Harness::new().await;
    let todo_id = seed_todo(&h).await;
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
    assert!(mine.get("execution").is_none(), "{mine}");
    assert_eq!(mine["status"], "draft");
    assert!(mine.get("detail_error").is_none(), "{mine}");
    let (detail_status, detail) = h
        .req(Method::GET, &format!("/api/executions/{exec_id}"), None)
        .await;
    assert_eq!(detail_status, 404, "{detail}");
    assert_eq!(detail["error"], "execution not found");
}

#[tokio::test]
async fn overview_ignores_execution_inspect_failures() {
    let h = Harness::new().await;
    let todo_id = seed_todo(&h).await;
    let exec_id = format!("project-{todo_id}");
    h.put_index(&exec_id, ExecutionKind::Project, ExecutionStatus::Idle)
        .await;
    h.node
        .set_inspect_reply(&exec_id, RpcReply::error(503, "node offline"));

    let (status, body) = h.req(Method::GET, "/api/project/overview", None).await;
    assert_eq!(status, 200, "{body}");
    let mine = body["backlog"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == json!(todo_id))
        .unwrap();
    assert_eq!(mine["status"], "draft");
    assert!(mine.get("detail_error").is_none(), "{mine}");
    let (detail_status, detail) = h
        .req(Method::GET, &format!("/api/executions/{exec_id}"), None)
        .await;
    assert_eq!(detail_status, 503, "{detail}");
    assert_eq!(detail["error"], "node offline");
}

#[tokio::test]
async fn todo_runs_degrade_to_an_empty_page_when_the_node_lost_the_run() {
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
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["runs"], json!([]));
    assert_eq!(body["next_version"], json!(null));
    assert_eq!(body["more"], json!(false));
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
async fn legacy_brain_todo_execute_rejects_without_default_agent() {
    let h = Harness::new().await;
    let todo_id = seed_todo_with_kind(&h, Some("brain")).await;

    let (status, body) = h
        .req(
            Method::POST,
            &format!("/api/project/todos/{todo_id}/execute"),
            Some(json!({})),
        )
        .await;
    assert_eq!(status, 409, "{body}");
    assert!(body.to_string().contains("migration required"));
    assert!(h
        .node
        .seen_commands()
        .iter()
        .all(|(_, action, _)| action != "execute"));
}

#[tokio::test]
async fn keyed_project_routing_rejects_changed_intent_after_unconfirmed_command() {
    let h = Harness::new().await;
    let todo = seed_todo_with_kind(&h, Some("agent")).await;
    let path = format!("/api/project/todos/{todo}");
    let (status, body) = h
        .req(Method::POST, &format!("{path}/plan"), Some(json!({})))
        .await;
    assert_eq!(status, 202, "{body}");
    let id = format!("project-{todo}");
    h.node
        .set_command(&id, "project-receipt", 404, json!({"error":"not accepted"}));
    h.node
        .set_command(&id, "execute", 503, json!({"error":"unconfirmed"}));
    let request = json!({"run_id":"prun-routing-release","reason":"original"});
    let (status, body) = h
        .req(
            Method::POST,
            &format!("{path}/execute"),
            Some(request.clone()),
        )
        .await;
    assert_eq!(status, 503, "{body}");
    let (status, body) = h
        .req(
            Method::POST,
            &format!("{path}/execute"),
            Some(json!({"run_id":"prun-routing-release","reason":"changed"})),
        )
        .await;
    assert_eq!(status, 409, "{body}");
    let (status, body) = h
        .req(Method::POST, &format!("{path}/execute"), Some(request))
        .await;
    assert_eq!(status, 503, "{body}");
    let commands: Vec<_> = h
        .node
        .seen_commands()
        .into_iter()
        .filter(|(_, action, _)| action == "execute")
        .collect();
    assert_eq!(commands.len(), 2);
    assert_eq!(commands[0].2, commands[1].2);
}
