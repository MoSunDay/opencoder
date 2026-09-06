use super::*;

#[tokio::test]
async fn project_plan_act_and_new_draft_stay_on_the_assigned_node() {
    let fleet = Fleet::new(2, mock()).await;
    let created = fleet
        .call(
            "POST",
            "/api/project/todos",
            json!({"title":"deliver change","draft":"first draft"}),
        )
        .await;
    assert_eq!(created.status, 200, "{:?}", created);
    let todo = created.body["id"].as_str().unwrap();
    let id = format!("project-{todo}");
    let pinned = fleet.nodes[1].registration().id;
    assert_eq!(
        fleet
            .call(
                "POST",
                &format!("/api/project/todos/{todo}/plan"),
                json!({"node_id":pinned})
            )
            .await
            .status,
        202
    );
    let detail = settled(&fleet.nodes[1], &id).await;
    assert_eq!(detail["execution"]["status"], "idle", "{detail}");
    assert!(detail["todo"]["plan_md"].is_string());
    let index = fleet.state.fleet.index(&id).await.unwrap().unwrap();
    assert_eq!(index.node_id, pinned);
    assert_eq!(
        fleet
            .call(
                "POST",
                &format!("/api/project/todos/{todo}/execute"),
                json!({})
            )
            .await
            .status,
        200
    );
    let detail = settled(&fleet.nodes[1], &id).await;
    assert_eq!(detail["execution"]["status"], "idle", "{detail}");
    assert!(detail["runs"].as_array().unwrap().len() >= 2);
    assert!(fleet
        .state
        .projects
        .get_todo(todo)
        .await
        .unwrap()
        .unwrap()
        .plan_md
        .is_none());
    let overview = fleet
        .call("GET", "/api/project/overview", Value::Null)
        .await;
    assert_eq!(overview.body["backlog"][0]["plan_md"], "node-owned answer");
    assert_eq!(
        fleet
            .call(
                "PATCH",
                &format!("/api/project/todos/{todo}"),
                json!({"draft":"second draft"})
            )
            .await
            .status,
        200
    );
    assert_eq!(
        fleet
            .call(
                "POST",
                &format!("/api/project/todos/{todo}/plan"),
                json!({})
            )
            .await
            .status,
        200
    );
    let detail = settled(&fleet.nodes[1], &id).await;
    assert_eq!(detail["todo"]["draft"], "second draft");
    assert_eq!(
        fleet
            .call(
                "PATCH",
                &format!("/api/project/todos/{todo}"),
                json!({"draft":"third draft"})
            )
            .await
            .status,
        200
    );
    assert_eq!(
        fleet
            .call(
                "POST",
                &format!("/api/executions/{id}/commands"),
                json!({"action":"plan","input":{"snapshot":{"todo":{"draft":"spoofed"}}}})
            )
            .await
            .status,
        200
    );
    let detail = settled(&fleet.nodes[1], &id).await;
    assert_eq!(detail["todo"]["draft"], "third draft");
    assert_eq!(
        fleet.state.fleet.index(&id).await.unwrap().unwrap().node_id,
        pinned
    );
    assert!(!fleet.nodes[0]
        .indexes()
        .await
        .unwrap()
        .iter()
        .any(|r| r.id == id));
    fleet.shutdown().await;
}

#[tokio::test]
async fn ordinary_team_stays_on_one_node_and_system_creation_is_retired() {
    let client = mock();
    let fleet = Fleet::new(2, client.clone()).await;
    let coordinator = fleet.nodes[0].registration().id;
    let remote_before: std::collections::HashSet<_> = fleet.nodes[1]
        .indexes()
        .await
        .unwrap()
        .into_iter()
        .map(|index| index.id)
        .collect();
    let saved = fleet
        .call(
            "POST",
            "/api/teams",
            json!({"name":"local-team","captain":"captain","members":[
                {"id":"captain","agent":"act","role":"coordinate"},
                {"id":"reviewer","agent":"act","role":"review"}
            ]}),
        )
        .await;
    assert_eq!(saved.status, 200, "{saved:?}");
    for text in [
        json!({"question":"review locally","participants":["reviewer"],"rationale":"review"})
            .to_string(),
        "local review answer".into(),
        "{\"summary\":\"all checked\",\"aligned\":true}".into(),
        "{\"complete\":true,\"final_summary\":\"team complete\"}".into(),
    ] {
        client.queue_script(vec![LlmEvent::Completed {
            text,
            tool_calls: vec![],
            usage: None,
        }]);
    }
    let reply = fleet
        .call(
            "POST",
            "/api/executions",
            json!({"id":"team-local","kind":"team","target":"local-team","node_id":coordinator,"input":{"prompt":"review"}}),
        )
        .await;
    assert_eq!(reply.status, 202, "{reply:?}");
    let detail = settled(&fleet.nodes[0], "team-local").await;
    assert_eq!(detail["execution"]["status"], "done", "{detail}");
    assert!(fleet.nodes[0]
        .indexes()
        .await
        .unwrap()
        .iter()
        .any(|index| index.id.starts_with("member-")));
    let dag = fleet
        .call(
            "POST",
            "/api/executions",
            json!({"id":"dag-local","kind":"dag","node_id":coordinator,"input":{"definition":{
                "name":"local-dag","steps":[{"name":"review","kind":{"type":"agent","prompt":"review locally"}}]
            }}}),
        )
        .await;
    assert_eq!(dag.status, 202, "{dag:?}");
    let dag_detail = settled(&fleet.nodes[0], "dag-local").await;
    assert_eq!(dag_detail["execution"]["status"], "done", "{dag_detail}");
    assert!(fleet.nodes[0]
        .indexes()
        .await
        .unwrap()
        .iter()
        .any(|index| index.id == "dag-local"));
    let remote_after: std::collections::HashSet<_> = fleet.nodes[1]
        .indexes()
        .await
        .unwrap()
        .into_iter()
        .map(|index| index.id)
        .collect();
    assert_eq!(remote_after, remote_before);

    let retired = fleet.call("POST", "/api/executions", json!({
        "id":"system-new","kind":"system","node_id":coordinator,"input":{"prompt":"query nodes"}
    })).await;
    assert_eq!(retired.status, 400, "{retired:?}");
    assert!(fleet
        .state
        .fleet
        .index("system-new")
        .await
        .unwrap()
        .is_none());
    fleet
        .state
        .fleet
        .put_definition(
            "team",
            "system",
            &json!({"name":"system","captain":"old","members":[]}),
        )
        .await
        .unwrap();
    let teams = fleet.call("GET", "/api/teams", Value::Null).await;
    assert!(teams.body["teams"]
        .as_array()
        .unwrap()
        .iter()
        .all(|team| team["name"] != "system"));
    assert_eq!(
        fleet
            .call(
                "POST",
                "/api/executions",
                json!({"id":"team-old-system","kind":"team","target":"system"})
            )
            .await
            .status,
        400
    );

    fleet
        .state
        .fleet
        .put_index(&opencoder_core::fleet::ExecutionIndex {
            id: "system-history".into(),
            created_at: 1,
            kind: opencoder_core::fleet::ExecutionKind::System,
            node_id: coordinator,
            status: opencoder_core::fleet::ExecutionStatus::Interrupted,
        })
        .await
        .unwrap();
    assert_eq!(
        fleet
            .call(
                "POST",
                "/api/executions/system-history/commands",
                json!({"action":"resume"})
            )
            .await
            .status,
        400
    );
    assert_eq!(
        fleet
            .call("GET", "/api/executions/system-history", Value::Null)
            .await
            .status,
        404
    );
    assert_eq!(
        fleet
            .call(
                "POST",
                "/api/executions/system-history/commands",
                json!({"action":"cancel"})
            )
            .await
            .status,
        404
    );
    fleet.shutdown().await;
}
