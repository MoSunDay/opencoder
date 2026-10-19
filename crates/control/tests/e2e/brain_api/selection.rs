use super::*;

#[tokio::test]
async fn library_and_tui_share_selection_while_hidden_ids_remain_valid() {
    let h = Harness::with_brain_kind().await;
    let root = opencoder_core::Config::load(&h.state.workdir)
        .unwrap()
        .agent
        .agents_dir
        .unwrap();
    let prompt = root.join("prompts/catalog-prompt");
    std::fs::create_dir_all(prompt.join("v1")).unwrap();
    std::fs::write(
        prompt.join("meta.json"),
        json!({"name":"catalog-prompt","current":1}).to_string(),
    )
    .unwrap();
    std::fs::write(
        prompt.join("v1/soul.md"),
        "Complete the requested generic task.",
    )
    .unwrap();
    let (status, body) = h
        .req(
            Method::POST,
            "/api/agents",
            Some(json!({"name":"catalog-worker","current":{"prompt":"catalog-prompt"}})),
        )
        .await;
    assert_eq!(status, 201, "{body}");
    let stored = seed_cap(&h, "registered purpose").await;
    let (status, body) = h
        .req(
            Method::PUT,
            &format!("/api/brain/capabilities/{stored}/target"),
            Some(json!({"kind":"agent","target":"catalog-worker"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, library) = h.req(Method::GET, "/api/brain/library", None).await;
    assert_eq!(status, 200, "{library}");
    let full = library["capabilities"].as_array().unwrap();
    assert!(full.iter().any(|cap| cap["id"] == "agent-catalog-worker"));
    let selected = library["selection_ids"].as_array().unwrap();
    assert!(selected.contains(&json!(stored)));
    assert!(!selected.contains(&json!("agent-catalog-worker")));
    assert!(selected.contains(&json!("builtin-agent-act")));
    let (status, tui) = h
        .req(Method::GET, "/api/tui/agent-capabilities", None)
        .await;
    assert_eq!(status, 200, "{tui}");
    let cards = tui["capabilities"].as_array().unwrap();
    assert!(cards.iter().any(|cap| cap["id"] == stored));
    assert!(!cards.iter().any(|cap| cap["id"] == "agent-catalog-worker"));
    let mut plan = super::super::layered_api::plan();
    plan["nodes"][0]["capability_id"] = json!("agent-catalog-worker");
    let (status,body)=h.req(Method::POST,"/api/brain/plan-defs",Some(json!({"id":"legacy-selection","version":1,"plan":plan,"changelog":"retain old binding","created_at":1}))).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["version"]["plan"]["nodes"][0]["capability_id"],
        "agent-catalog-worker"
    );
    let (status, body) = h
        .req(
            Method::PUT,
            &format!("/api/brain/capabilities/{stored}/target"),
            Some(json!({"kind":"agent","target":"catalog-worker","required_inputs":["source"]})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (_, library) = h.req(Method::GET, "/api/brain/library", None).await;
    assert!(library["selection_ids"]
        .as_array()
        .unwrap()
        .contains(&json!("agent-catalog-worker")));
}
