use super::*;

#[tokio::test]
async fn dispatch_unkeyed_creates_execution_and_keyed_is_idempotent() {
    let h = Harness::new().await;
    let cap = seed_cap(&h, TOPIC_A).await;
    let (status, _) = h
        .req(
            Method::PUT,
            &format!("/api/brain/capabilities/{cap}/target"),
            Some(json!({"kind": "agent", "target": "act"})),
        )
        .await;
    assert_eq!(status, 200);

    // Each planner round-trip consumes one scripted LLM reply: the unkeyed
    // dispatch and the keyed first call each plan once; the keyed replay is
    // served from the durable receipt without consulting the LLM.
    queue_plan(&h, &cap);
    queue_plan(&h, &cap);

    // Unkeyed: full decision + execution in one call.
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/dispatch",
            Some(json!({"situation": TOPIC_A, "node_id": "node-e2e"})),
        )
        .await;
    assert_eq!(status, 202, "{body}");
    assert_eq!(body["capability_id"], json!(cap));
    assert_eq!(body["execution"]["kind"], json!("agent"));
    assert_eq!(body["execution"]["node_id"], json!("node-e2e"));

    // Keyed first call mints agent-<request_id>; replay returns the same
    // execution through the node's durable receipt.
    let (status, first) = h
        .req(
            Method::POST,
            "/api/brain/dispatch",
            Some(json!({"situation": TOPIC_A, "request_id": "r1"})),
        )
        .await;
    assert_eq!(status, 202, "{first}");
    assert_eq!(first["execution"]["id"], json!("agent-r1"));
    let (status, replay) = h
        .req(
            Method::POST,
            "/api/brain/dispatch",
            Some(json!({"situation": TOPIC_A, "request_id": "r1"})),
        )
        .await;
    assert_eq!(status, 202, "{replay}");
    assert_eq!(replay["execution"]["id"], json!("agent-r1"));
    assert_eq!(
        replay["execution"]["created_at"],
        first["execution"]["created_at"]
    );

    // Validation: empty situation / bad request_id.
    for bad in [
        json!({"situation": ""}),
        json!({"situation": "x", "request_id": "bad id!"}),
        json!({"situation": "x", "id": "agent-q", "request_id": "q2"}),
    ] {
        let (status, body) = h.req(Method::POST, "/api/brain/dispatch", Some(bad)).await;
        assert_eq!(status, 400, "{body}");
    }
}

#[tokio::test]
async fn dispatch_without_target_binding_uses_the_default_agent() {
    let h = Harness::new().await;
    let cap = seed_cap(&h, "unbound capability").await;
    queue_plan(&h, &cap);
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/dispatch",
            Some(json!({"situation": "unbound capability"})),
        )
        .await;
    assert_eq!(status, 202, "{body}");
    assert_eq!(body["capability_id"], json!(cap));
    assert_eq!(body["execution"]["kind"], json!("agent"));
    assert_eq!(body["execution"]["node_id"], json!(h.node.id));
}

// Token sanity: brain endpoints are behind the bearer like everything else.
