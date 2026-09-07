use super::*;
use opencoder_core::fleet::NodeOperation;

#[tokio::test]
async fn empty_library_executes_on_selected_node_and_retries_the_same_receipt() {
    let h = Harness::new().await;
    let request = json!({"situation":"finish the requested task", "node_id":h.node.id, "request_id":"first-use"});
    let (status, first) = h
        .req(Method::POST, "/api/brain/dispatch", Some(request.clone()))
        .await;
    assert_eq!(status, 202, "{first}");
    assert_eq!(first["route"], "default_agent");
    assert_eq!(first["execution"]["id"], "agent-first-use");
    assert_eq!(first["execution"]["node_id"], json!(h.node.id));
    assert!(first["capability_id"].is_null());
    assert!(first["plan_id"].is_null());
    assert_eq!(first["planned_fresh"], false);
    let index = h
        .state
        .fleet
        .index("agent-first-use")
        .await
        .unwrap()
        .unwrap();
    let accepted = h
        .state
        .hub
        .call(
            &h.node.id,
            NodeOperation::AcceptedRequest {
                execution: index.execution_ref(),
            },
        )
        .await;
    assert_eq!(accepted.status, 200);
    assert_eq!(accepted.body["receipt"]["decision"]["target"], "act");
    assert_eq!(
        accepted.body["receipt"]["intent"]["situation"],
        "finish the requested task"
    );
    // A library created after acceptance cannot re-route a retried request.
    seed_cap(&h, "new knowledge").await;
    let (status, replay) = h
        .req(Method::POST, "/api/brain/dispatch", Some(request))
        .await;
    assert_eq!(status, 202, "{replay}");
    assert_eq!(replay, first);
    assert_eq!(h.node.journal_ids(), vec!["agent-first-use"]);
    let (status, _) = h
        .req(
            Method::POST,
            "/api/brain/dispatch",
            Some(json!({
                "situation":"changed task", "node_id":h.node.id, "request_id":"first-use"
            })),
        )
        .await;
    assert_eq!(status, 409);
}

#[tokio::test]
async fn empty_library_respects_explicit_node_and_plan_errors() {
    let h = Harness::new().await;
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/dispatch",
            Some(json!({
                "situation":"run here", "node_id":"node-missing", "request_id":"wrong-node"
            })),
        )
        .await;
    assert!(!status.is_success(), "{body}");
    assert!(h.node.journal_ids().is_empty());
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/dispatch",
            Some(json!({
                "situation":"run this plan", "node_id":h.node.id, "plan_id":"missing-plan"
            })),
        )
        .await;
    assert_eq!(status, 404, "{body}");
    assert!(h.node.journal_ids().is_empty());
}

#[tokio::test]
async fn default_unkeyed_dispatch_preserves_the_requested_execution_id() {
    let h = Harness::new().await;
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/dispatch",
            Some(json!({
                "situation":"run directly", "node_id":h.node.id, "id":"agent-explicit-default"
            })),
        )
        .await;
    assert_eq!(status, 202, "{body}");
    assert_eq!(body["execution"]["id"], "agent-explicit-default");
    assert_eq!(body["route"], "default_agent");
}

#[tokio::test]
async fn configured_library_planner_failure_does_not_start_a_default_execution() {
    let h = Harness::new().await;
    seed_cap(&h, "requires a planner").await;
    h.mock_llm.queue_script(vec![
        opencoder_llm::LlmEvent::TextDelta("not a decision tree".into()),
        opencoder_llm::LlmEvent::Completed {
            text: "not a decision tree".into(),
            tool_calls: Vec::new(),
            usage: None,
        },
    ]);
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/dispatch",
            Some(json!({
                "situation":"plan this", "node_id":h.node.id, "request_id":"planner-error"
            })),
        )
        .await;
    assert_eq!(status, 502, "{body}");
    assert!(h.node.journal_ids().is_empty());
}
