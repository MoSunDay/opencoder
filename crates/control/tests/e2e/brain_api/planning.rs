use super::*;

#[tokio::test]
async fn plan_lifecycle_with_scripted_planner() {
    let h = Harness::new().await;
    let cap_a = seed_cap(&h, TOPIC_A).await;
    let cap_b = seed_cap(&h, "write unit tests").await;
    let reply = format!(
        "{{\"threshold\":{THRESH},\"root\":{{\"id\":\"b1\",\"kind\":\"branch\",\"topic\":\"{TOPIC_A}\",\"yes\":{{\"id\":\"l1\",\"kind\":\"leaf\",\"capability_id\":\"{cap_a}\",\"reason\":\"db work\"}},\"no\":{{\"id\":\"l2\",\"kind\":\"leaf\",\"capability_id\":\"{cap_b}\",\"reason\":\"test work\"}}}}}}"
    );
    h.mock_llm.queue_script(vec![
        opencoder_llm::LlmEvent::TextDelta(reply.clone()),
        opencoder_llm::LlmEvent::Completed {
            text: reply,
            tool_calls: Vec::new(),
            usage: None,
        },
    ]);

    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/plans",
            Some(json!({"situation": TOPIC_A})),
        )
        .await;
    assert_eq!(status, 201, "{body}");
    let plan_id = body["plan"]["id"].as_str().unwrap().to_string();
    assert!(plan_id.starts_with("brain-plan-"), "{body}");
    assert_eq!(body["tree"]["root"]["kind"], json!("branch"));

    let (status, body) = h
        .req(Method::GET, &format!("/api/brain/plans/{plan_id}"), None)
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["plan"]["id"], json!(plan_id));
    let (status, body) = h
        .req(Method::GET, "/api/brain/plans/brain-plan-none", None)
        .await;
    assert_eq!(status, 404, "{body}");

    // Preview routes the branch topic to the yes leaf without dispatching.
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/preview",
            Some(json!({"situation": TOPIC_A, "plan_id": plan_id})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["capability_id"], json!(cap_a));
    assert!(!body["path"].as_array().unwrap().is_empty(), "{body}");
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/preview",
            Some(json!({"situation": ""})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
}

/// Plans admission gate + planner fault surface + dynamic preview: blank
/// situations are 400 before any LLM work, an unparseable planner reply is
/// a 502, an unknown plan_id in preview is 404, a preview without plan_id
/// plans fresh (planned_fresh) through the scripted planner, and a drained
/// server refuses new plans with 503.
#[tokio::test]
async fn plans_gates_planner_faults_and_dynamic_preview() {
    let h = Harness::new().await;
    let cap = seed_cap(&h, TOPIC_A).await;

    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/plans",
            Some(json!({"situation": "  "})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"], json!("situation must not be empty"));

    // Garbage planner text (valid script, non-JSON payload) → 502.
    h.mock_llm.queue_script(vec![
        opencoder_llm::LlmEvent::TextDelta("definitely not a decision tree".into()),
        opencoder_llm::LlmEvent::Completed {
            text: "definitely not a decision tree".into(),
            tool_calls: Vec::new(),
            usage: None,
        },
    ]);
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/plans",
            Some(json!({"situation": TOPIC_A})),
        )
        .await;
    assert_eq!(status, 502, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("planner reply unparseable"),
        "{body}"
    );

    // Unknown plan in preview: 404 without consulting the LLM.
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/preview",
            Some(json!({"situation": TOPIC_A, "plan_id": "brain-plan-none"})),
        )
        .await;
    assert_eq!(status, 404, "{body}");

    // Dynamic mode (no plan_id): plans fresh through the scripted planner.
    queue_plan(&h, &cap);
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/preview",
            Some(json!({"situation": TOPIC_A})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["capability_id"], json!(cap));
    assert_eq!(body["planned_fresh"], json!(true), "{body}");

    // Drained server: the plans admission gate refuses with 503.
    let (status, _) = h.req(Method::POST, "/api/admin/drain", None).await;
    assert_eq!(status, 200);
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/plans",
            Some(json!({"situation": "post drain"})),
        )
        .await;
    assert_eq!(status, 503, "{body}");
    assert_eq!(body["error"], json!("server admission is frozen"));
}
