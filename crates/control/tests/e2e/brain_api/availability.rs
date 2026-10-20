use super::*;

async fn agent(h: &Harness) -> String {
    let name = "profile-worker";
    let (status, body) = h
        .req(
            Method::POST,
            "/api/agents",
            Some(json!({"name":name,"harness":"codex",
                "harness_profile":"task-profile","current":{}})),
        )
        .await;
    assert_eq!(status, 201, "{body}");
    let root = opencoder_core::Config::load(&h.state.workdir)
        .unwrap()
        .agent
        .agents_dir
        .unwrap();
    let prompt = root.join("prompts/profile-prompt");
    std::fs::create_dir_all(prompt.join("v1")).unwrap();
    std::fs::write(prompt.join("meta.json"), r#"{"current":1}"#).unwrap();
    std::fs::write(prompt.join("v1/soul.md"), "Complete this task.").unwrap();
    let (status, body) = h
        .req(
            Method::PUT,
            &format!("/api/agents/{name}"),
            Some(
                json!({"name":name,"harness":"codex","harness_profile":"task-profile",
                "current":{"prompt":"profile-prompt"}}),
            ),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    name.into()
}

#[tokio::test]
async fn missing_profiles_are_visible_unavailable_and_recover_without_recreating_agents() {
    let h = Harness::with_brain_kind().await;
    h.node
        .set_maintenance("execution_profiles", 200, json!({"agent":[],"operator":[]}));
    let name = agent(&h).await;
    let id = format!("agent-{name}");
    let (_, library) = h.req(Method::GET, "/api/brain/library", None).await;
    let entry = library["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == id)
        .unwrap();
    assert!(entry["unavailable_reason"]
        .as_str()
        .unwrap()
        .contains("task-profile"));
    assert!(!library["selection_ids"]
        .as_array()
        .unwrap()
        .contains(&json!(id)));
    let mut plan = super::super::layered_api::plan();
    plan["nodes"][0]["capability_id"] = json!(id);
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/plan-defs",
            Some(json!({"id":"profile-plan","version":1,"plan":plan,
            "changelog":"profile availability","created_at":1})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert!(body.to_string().contains("task-profile"));
    let (status, body) = h
        .req(
            Method::PUT,
            "/api/harnesses/codex/profiles/task-profile",
            Some(json!({"model":"fixture-model"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (_, library) = h.req(Method::GET, "/api/brain/library", None).await;
    assert!(library["selection_ids"]
        .as_array()
        .unwrap()
        .contains(&json!(id)));
    let (_, meta) = h
        .req(Method::GET, &format!("/api/agents/{name}/meta"), None)
        .await;
    assert_eq!(meta["meta"]["harness_profile"], "task-profile");
}

#[tokio::test]
async fn private_profiles_and_unknown_legacy_nodes_keep_existing_admission_behavior() {
    let h = Harness::with_brain_kind().await;
    let name = agent(&h).await;
    let id = json!(format!("agent-{name}"));
    for (status, body) in [
        (400, json!({"error":"unknown maintenance operation"})),
        (200, json!({"agent":["task-profile"],"operator":[]})),
    ] {
        h.node.set_maintenance("execution_profiles", status, body);
        let (status, library) = h.req(Method::GET, "/api/brain/library", None).await;
        assert_eq!(status, 200, "{library}");
        assert!(library["selection_ids"].as_array().unwrap().contains(&id));
    }
    let stored = seed_cap(&h, "operator with its own configuration").await;
    let (status, body) = h
        .req(
            Method::PUT,
            &format!("/api/brain/capabilities/{stored}/target"),
            Some(json!({"kind":"operator","target":name})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (_, library) = h.req(Method::GET, "/api/brain/library", None).await;
    assert!(!library["selection_ids"]
        .as_array()
        .unwrap()
        .contains(&json!(stored)));
    assert!(library["selection_ids"].as_array().unwrap().contains(&id));
}
