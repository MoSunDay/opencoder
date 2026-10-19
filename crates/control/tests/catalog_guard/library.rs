use super::support::{spec, Harness};
use serde_json::{json, Value};

async fn register(h: &Harness, summary: &str) -> String {
    let result = h
        .call(
            "POST",
            "/api/brain/capabilities",
            json!({"capability_type":"workflow","summary":summary,
        "input_desc":"explicit inputs","output_desc":"explicit outputs","eng_inputs":["example"]}),
        )
        .await;
    assert_eq!(result.0, 201, "{}", result.1);
    result.1["capability"]["id"].as_str().unwrap().into()
}

#[tokio::test]
async fn library_collapses_stored_and_discovered_sources_but_keeps_raw_targets() {
    let h = Harness::new().await;
    h.call("POST", "/api/dag/defs", spec("workflow")).await;
    let id = register(&h, "authored workflow").await;
    assert_eq!(
        h.call(
            "PUT",
            &format!("/api/brain/capabilities/{id}/target"),
            json!({"kind":"dag","target":"workflow"})
        )
        .await
        .0,
        200
    );
    let library = h.call("GET", "/api/brain/library", Value::Null).await;
    assert_eq!(library.0, 200, "{}", library.1);
    let entries: Vec<_> = library.1["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| {
            v["target"] == "workflow"
                && library.1["selection_ids"]
                    .as_array()
                    .unwrap()
                    .contains(&v["id"])
        })
        .collect();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["id"], id);
    assert_eq!(entries[0]["input_desc"], "explicit inputs");
    assert_eq!(entries[0]["equivalent_sources"][0]["id"], "dag-workflow");
    assert!(library.1["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v["id"] == "dag-workflow"));
    assert_eq!(
        h.call("GET", "/api/dag/defs/workflow", Value::Null).await.0,
        200
    );
    assert_eq!(
        h.call("GET", &format!("/api/brain/capabilities/{id}"), Value::Null)
            .await
            .0,
        200
    );
    let duplicate = register(&h, "another description").await;
    assert_eq!(
        h.call(
            "PUT",
            &format!("/api/brain/capabilities/{duplicate}/target"),
            json!({"kind":"dag","target":" workflow "})
        )
        .await
        .0,
        409
    );
}

#[tokio::test]
async fn cross_name_agents_merge_only_with_the_same_actual_resource_bindings() {
    use base64::Engine;
    let h = Harness::new().await;
    let saved=h.call("POST","/api/agents/resources/prompts",json!({"name":"shared","files":[{"path":"soul.md","content_b64":base64::engine::general_purpose::STANDARD.encode("Shared instructions")}]})).await;
    assert_eq!(saved.0, 200, "{}", saved.1);
    for name in ["one", "two"] {
        let result = h
            .call(
                "POST",
                "/api/agents",
                json!({"name":name,"current":{"prompt":"shared"}}),
            )
            .await;
        assert_eq!(result.0, 201, "{}", result.1);
    }
    let listed = h.call("GET", "/api/brain/library", Value::Null).await;
    let entries: Vec<_> = listed.1["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| {
            matches!(v["target"].as_str(), Some("one" | "two"))
                && listed.1["selection_ids"]
                    .as_array()
                    .unwrap()
                    .contains(&v["id"])
        })
        .collect();
    assert_eq!(entries.len(), 1, "{}", listed.1);
    assert_eq!(
        entries[0]["equivalent_sources"].as_array().unwrap().len(),
        1
    );
    // A different runtime profile is meaningful even when prompt/tools match.
    let path = h.dir.path().join("agents/two/meta.json");
    let mut meta: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    meta["harness_profile"] = json!("different-profile");
    std::fs::write(path, meta.to_string()).unwrap();
    let listed = h.call("GET", "/api/brain/library", Value::Null).await;
    assert_eq!(
        listed.1["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| matches!(v["target"].as_str(), Some("one" | "two"))
                && listed.1["selection_ids"]
                    .as_array()
                    .unwrap()
                    .contains(&v["id"]))
            .count(),
        2
    );
}
