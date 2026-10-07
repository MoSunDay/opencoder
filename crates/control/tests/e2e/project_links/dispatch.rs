use crate::support::Harness;
use opencoder_core::fleet::{ExecutionKind, ExecutionStatus, RpcReply};
use reqwest::Method;
use serde_json::{json, Value};

async fn todo(h: &Harness) -> String {
    let (status, row) = h
        .req(
            Method::POST,
            "/api/project/todos",
            Some(json!({
                "title":"shared project task","draft":"work","capability_id":"builtin-agent-act"
            })),
        )
        .await;
    assert_eq!(status, 200, "{row}");
    format!("/api/project/todos/{}", row["id"].as_str().unwrap())
}

#[tokio::test]
async fn dispatch_retry_freezes_capability_and_rejects_reuse_with_other_input() {
    let h = Harness::new().await;
    let path = todo(&h).await;
    let dispatch_path = format!("{path}/dispatch");
    let cap =
        json!({"id":"employee-cap","kind":"agent","target":"act","definition":{"name":"act"}});
    h.state
        .fleet
        .put_definition("brain_capability", "employee-cap", &cap)
        .await
        .unwrap();
    let body = json!({"execution_id":"agent-project-retry","capability_id":"employee-cap","input":{"prompt":"fixed work"}});
    let (status, reply) = h
        .dispatch(Method::POST, &dispatch_path, Some(body.clone()))
        .await;
    assert_eq!(status, 202, "{reply}");
    h.state
        .fleet
        .put_definition(
            "brain_capability",
            "employee-cap",
            &json!({
                "id":"employee-cap","kind":"agent","target":"different","definition":{}
            }),
        )
        .await
        .unwrap();
    let (status, again) = h
        .req(Method::POST, &dispatch_path, Some(body.clone()))
        .await;
    assert_eq!(status, 202, "{again}");
    assert_eq!(h.node.journal_ids(), ["agent-project-retry"]);
    assert_eq!(
        h.node.journal_request("agent-project-retry").unwrap()["target"],
        "act"
    );
    let (_, links) = h
        .req(Method::GET, &format!("{path}/executions"), None)
        .await;
    assert_eq!(links["assignments"].as_array().unwrap().len(), 1);
    assert_eq!(links["assignments"][0]["capability_id"], "employee-cap");
    let mut changed = body;
    changed["input"]["prompt"] = json!("different work");
    assert_eq!(
        h.req(Method::POST, &format!("{path}/dispatch"), Some(changed))
            .await
            .0,
        409
    );
    assert_eq!(
        h.req(
            Method::POST,
            &format!("{path}/executions"),
            Some(json!({
                "execution_id":"agent-project-retry","capability_id":"builtin-operator"
            }))
        )
        .await
        .0,
        409
    );
    assert_eq!(h.req(Method::DELETE, &path, None).await.0, 200);
    assert!(h
        .state
        .fleet
        .index("agent-project-retry")
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn separate_operators_can_work_on_the_same_todo_without_an_assignee_lock() {
    let h = Harness::new().await;
    let path = todo(&h).await;
    let dispatch_path = format!("{path}/dispatch");
    let (a, b) = tokio::join!(
        h.dispatch(Method::POST, &dispatch_path, Some(json!({"execution_id":"agent-project-a","capability_id":"builtin-agent-act","input":{"prompt":"first"}}))),
        h.dispatch(Method::POST, &dispatch_path, Some(json!({"execution_id":"operator-project-b","capability_id":"builtin-operator","input":{"prompt":"second"}})))
    );
    assert_eq!(a.0, 202, "{a:?}");
    assert_eq!(b.0, 202, "{b:?}");
    let (_, links) = h
        .req(Method::GET, &format!("{path}/executions"), None)
        .await;
    assert_eq!(links["assignments"].as_array().unwrap().len(), 2);
    let (_, row) = h.req(Method::GET, &path, None).await;
    assert_ne!(row["board_status"], "done");
}

#[tokio::test]
async fn results_are_read_from_the_owner_and_report_node_failures_without_cached_success() {
    let h = Harness::new().await;
    let path = todo(&h).await;
    h.put_index(
        "agent-live-result",
        ExecutionKind::Agent,
        ExecutionStatus::Done,
    )
    .await;
    let (_, link) = h
        .req(
            Method::POST,
            &format!("{path}/executions"),
            Some(json!({"execution_id":"agent-live-result"})),
        )
        .await;
    assert_eq!(link["execution_id"], "agent-live-result");
    for text in ["first answer", "updated answer"] {
        h.node.set_inspect(
            "agent-live-result",
            json!({"execution":{"status":"done"},"result":{"output_text":text}}),
        );
        let (status, body) = h
            .req(
                Method::GET,
                "/api/executions/agent-live-result/result",
                None,
            )
            .await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["summary"], text);
    }
    h.node
        .set_inspect_reply("agent-live-result", RpcReply::error(503, "owner offline"));
    assert_eq!(
        h.req(
            Method::GET,
            "/api/executions/agent-live-result/result",
            None
        )
        .await
        .0,
        503
    );
    let (_, links) = h
        .req(Method::GET, &format!("{path}/executions"), None)
        .await;
    assert!(links["assignments"][0].get("result_md").is_none());
    assert!(links["assignments"][0].get("sync_state").is_none());
}

#[tokio::test]
async fn shared_execution_guidance_keeps_each_human_input_id_and_the_same_node() {
    let h = Harness::new().await;
    h.put_index(
        "agent-shared",
        ExecutionKind::Agent,
        ExecutionStatus::Running,
    )
    .await;
    h.node
        .set_command("agent-shared", "steer", 202, json!({"ok":true}));
    // Both identities belong to this test's new temporary database.
    let mut tokens = Vec::new();
    for name in ["human-a", "human-b"] {
        let (status, body) = h
            .req(
                Method::POST,
                "/api/users",
                Some(json!({"name":name,"role":"user"})),
            )
            .await;
        assert_eq!(status, 200);
        tokens.push(body["token"].as_str().unwrap().to_owned());
    }
    h.node.set_inspect(
        "agent-shared",
        json!({"execution":{"id":"agent-shared"},"context":"same node session"}),
    );
    for (person, id) in [
        (0, "input-human-a"),
        (1, "input-human-b"),
        (0, "input-human-a"),
    ] {
        let response = h
            .req_raw(
                Method::POST,
                "/api/executions/agent-shared/commands",
                Some(json!({"action":"steer","input":{"prompt":"guidance","input_id":id}})),
                Some(&tokens[person]),
            )
            .await;
        assert_eq!(response.status(), 202);
        let detail: Value = h
            .req_raw(
                Method::GET,
                "/api/executions/agent-shared",
                None,
                Some(&tokens[person]),
            )
            .await
            .json()
            .await
            .unwrap();
        assert_eq!(detail["context"], "same node session");
    }
    let seen = h.node.seen_commands();
    assert!(seen
        .iter()
        .all(|(id, action, _)| id == "agent-shared" && action == "steer"));
    assert_eq!(
        seen.iter()
            .map(|(_, _, value)| value["input_id"].clone())
            .collect::<Vec<Value>>(),
        vec![
            json!("input-human-a"),
            json!("input-human-b"),
            json!("input-human-a")
        ]
    );
}
