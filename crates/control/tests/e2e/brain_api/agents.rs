use super::*;

/// GET /api/brain/agents aggregates agent-bound capabilities only: team
/// bindings and unbound capabilities are excluded from every group.
#[tokio::test]
async fn agents_lists_only_agent_bound_capabilities() {
    let h = Harness::new().await;
    let bound = seed_cap(&h, "agent bound capability").await;
    let team_bound = seed_cap(&h, "team bound capability").await;
    seed_cap(&h, "unbound capability").await;

    let (status, body) = h
        .req(
            Method::PUT,
            &format!("/api/brain/capabilities/{bound}/target"),
            Some(json!({"kind": "agent", "target": "act"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = h
        .req(
            Method::PUT,
            &format!("/api/brain/capabilities/{team_bound}/target"),
            Some(json!({"kind": "team", "target": "crew"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");

    let (status, body) = h.req(Method::GET, "/api/brain/agents", None).await;
    assert_eq!(status, 200, "{body}");
    let agents = body["agents"].as_array().unwrap();
    assert_eq!(
        agents.len(),
        1,
        "team-bound and unbound are excluded: {body}"
    );
    assert_eq!(agents[0]["agent"], json!("act"));
    let capabilities = agents[0]["capabilities"].as_array().unwrap();
    assert_eq!(capabilities.len(), 1, "{body}");
    assert_eq!(capabilities[0]["id"], json!(bound));
    assert_eq!(capabilities[0]["summary"], json!("agent bound capability"));
}
