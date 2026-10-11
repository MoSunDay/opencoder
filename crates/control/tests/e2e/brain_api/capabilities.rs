use super::*;

#[tokio::test]
async fn deleting_capability_preserves_legacy_saved_plan_references() {
    let h = Harness::new().await;
    let id = seed_cap(&h, "legacy plan source").await;
    let saved = serde_json::from_value(json!({
        "id":"legacy-consumer", "version":1, "changelog":"original", "created_at":1,
        "plan":{"schema_version":6,"title":"Legacy consumer",
            "nodes":[{"node_id":"old-node","capability_ids":[id]}]}
    }))
    .unwrap();
    h.state
        .fleet
        .save_brain_plan_document(&saved)
        .await
        .unwrap();
    let endpoint = format!("/api/brain/capabilities/{id}");
    let (status, body) = h.req(Method::DELETE, &endpoint, None).await;
    assert_eq!(status, 409, "{body}");
    assert!(body["error"]
        .as_str()
        .unwrap()
        .contains("plan:legacy-consumer@1"));
    assert_eq!(h.req(Method::GET, &endpoint, None).await.0, 200);
}

#[tokio::test]
async fn deleting_capability_preserves_saved_and_active_consumers() {
    let h = Harness::with_brain_kind().await;
    super::super::layered_api::advertise_v4(&h);
    let id = seed_cap(&h, "retained capability").await;
    let (status, body) = h
        .req(
            Method::PUT,
            &format!("/api/brain/capabilities/{id}/target"),
            Some(json!({"kind":"agent","target":"act"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let mut plan = super::super::layered_api::plan();
    plan["nodes"][0]["capability_id"] = json!(id);
    // The oldest version must still protect its source beyond the first page.
    for version in 1..=21 {
        let saved = json!({"id":"consumer","version":version,"plan":plan,
            "changelog":"consumer","created_at":1});
        let (status, body) = h
            .req(Method::POST, "/api/brain/plan-defs", Some(saved))
            .await;
        assert_eq!(status, 200, "{body}");
        plan["nodes"][0]["capability_id"] = json!("builtin-agent-act");
    }
    let endpoint = format!("/api/brain/capabilities/{id}");
    let (status, body) = h.req(Method::DELETE, &endpoint, None).await;
    assert_eq!(status, 409, "{body}");
    assert!(body["error"].as_str().unwrap().contains("plan:consumer@1"));
    assert_eq!(
        h.req(Method::DELETE, "/api/brain/plan-defs/consumer", None)
            .await
            .0,
        200
    );
    plan["nodes"][0]["capability_id"] = json!(id);
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/runs",
            Some(json!({"id":"brain-cap-consumer","schema_version":7,"plan":plan,"inputs":{}})),
        )
        .await;
    assert_eq!(status, 202, "{body}");
    let (status, body) = h.req(Method::DELETE, &endpoint, None).await;
    assert_eq!(status, 409, "{body}");
    assert!(body["error"]
        .as_str()
        .unwrap()
        .contains("run:brain-cap-consumer"));
    assert_eq!(h.req(Method::GET, &endpoint, None).await.0, 200);
    h.report_status(
        "brain-cap-consumer",
        opencoder_core::fleet::ExecutionStatus::Done,
    )
    .await;
    let (status, body) = h.req(Method::DELETE, &endpoint, None).await;
    assert_eq!(status, 200, "{body}");
    assert!(h
        .state
        .fleet
        .assignment("brain-cap-consumer")
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn capability_crud_search_and_target_binding() {
    let h = Harness::new().await;
    let id = seed_cap(&h, TOPIC_A).await;

    let (status, body) = h.req(Method::GET, "/api/brain/capabilities", None).await;
    assert_eq!(status, 200, "{body}");
    assert!(body["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["capability"]["id"] == json!(id)));

    let (status, body) = h
        .req(Method::GET, &format!("/api/brain/capabilities/{id}"), None)
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["capability"]["summary"], json!(TOPIC_A));
    let (status, body) = h
        .req(Method::GET, "/api/brain/capabilities/cap-none", None)
        .await;
    assert_eq!(status, 404, "{body}");

    let updated = capability_payload("updated summary");
    let (status, body) = h
        .req(
            Method::PUT,
            &format!("/api/brain/capabilities/{id}"),
            Some(updated),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["capability"]["summary"], json!("updated summary"));

    // Exact-text search hits the (re-embedded) capability.
    let typed: opencoder_brain::CapabilityInput =
        serde_json::from_value(capability_payload("updated summary")).unwrap();
    let composed = opencoder_brain::domain::compose_embed_text(&typed);
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/search",
            Some(json!({"query": composed, "k": 5})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert!(
        body["hits"]
            .as_array()
            .unwrap()
            .iter()
            .any(|hit| hit["capability"]["id"] == json!(id)),
        "{body}"
    );
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/search",
            Some(json!({"query": "  "})),
        )
        .await;
    assert_eq!(status, 400, "{body}");

    // Unbound target is null; binding persists and validates.
    let (status, body) = h
        .req(
            Method::GET,
            &format!("/api/brain/capabilities/{id}/target"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["target"], json!(null));
    let (status, body) = h
        .req(
            Method::PUT,
            &format!("/api/brain/capabilities/{id}/target"),
            Some(json!({"kind": "agent", "target": "act"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["kind"], json!("agent"));
    let (status, body) = h
        .req(
            Method::PUT,
            &format!("/api/brain/capabilities/{id}/target"),
            Some(json!({"kind": "bogus", "target": "x"})),
        )
        .await;
    // Unknown kind fails CapabilityTarget deserialization → axum 422.
    assert_eq!(status, 422, "{body}");
    let (status, body) = h
        .req(
            Method::PUT,
            "/api/brain/capabilities/cap-none/target",
            Some(json!({"kind": "agent", "target": "act"})),
        )
        .await;
    assert_eq!(status, 404, "{body}");

    h.state
        .fleet
        .put_definition("brain_capability_meta", &id, &json!({"maturity":"stable"}))
        .await
        .unwrap();
    let (status, body) = h
        .req(
            Method::DELETE,
            &format!("/api/brain/capabilities/{id}"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["deleted"], json!(id));
    let (status, body) = h
        .req(
            Method::GET,
            &format!("/api/brain/capabilities/{id}/target"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["target"], json!(null));
    assert!(h
        .state
        .fleet
        .definition("brain_capability_meta", &id)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn brain_requires_bearer() {
    let h = Harness::new().await;
    let resp = h
        .req_raw(
            Method::GET,
            "/api/brain/capabilities",
            None,
            Some(&format!("not-{TOKEN}")),
        )
        .await;
    assert_eq!(resp.status(), 401);
}

// ─── extra coverage: validation edges, search k policy, target guard ───

/// Field-level validation rejects blank summaries and oversized exemplar
/// inputs (both the per-capacity entry count and the per-entry length);
/// unknown ids are a 404 for update and delete alike.
#[tokio::test]
async fn capability_validation_edges_and_unknown_ids() {
    let h = Harness::new().await;

    let mut blank = capability_payload("   ");
    blank["summary"] = json!("   ");
    let (status, body) = h
        .req(Method::POST, "/api/brain/capabilities", Some(blank))
        .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"], json!("summary must not be empty"));

    // eng_inputs capacity is 64 entries; 65 is refused before embedding.
    let mut too_many = capability_payload("too many exemplars");
    too_many["eng_inputs"] = json!(vec!["exemplar"; 65]);
    let (status, body) = h
        .req(Method::POST, "/api/brain/capabilities", Some(too_many))
        .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"], json!("eng_inputs exceeds 64 entries"));

    // One entry above the 4000-char per-entry cap is refused too.
    let mut too_long = capability_payload("one huge exemplar");
    too_long["eng_inputs"] = json!(["x".repeat(4001)]);
    let (status, body) = h
        .req(Method::POST, "/api/brain/capabilities", Some(too_long))
        .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"], json!("eng_inputs[0] exceeds 4000 chars"));

    let valid = capability_payload("valid for update probes");
    let (status, body) = h
        .req(
            Method::PUT,
            "/api/brain/capabilities/cap-unknown",
            Some(valid.clone()),
        )
        .await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(
        body["error"],
        json!("brain capability not found: cap-unknown")
    );
    let (status, body) = h
        .req(Method::DELETE, "/api/brain/capabilities/cap-unknown", None)
        .await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(
        body["error"],
        json!("brain capability not found: cap-unknown")
    );
}

/// `k` policy on search: omitted → default 10, absurd values clamp to the
/// hard ceiling of 50 (the store LIMIT), and every hit carries the
/// capability record plus its vector distance.
#[tokio::test]
async fn search_k_default_and_clamp() {
    let h = Harness::new().await;
    for i in 0..55 {
        seed_cap(&h, &format!("bulk capability {i}")).await;
    }

    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/search",
            Some(json!({"query": "bulk capability"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let hits = body["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 10, "default k is 10: {body}");
    assert!(
        hits.iter()
            .all(|hit| hit["capability"]["id"].as_str().is_some()
                && hit["distance"].as_f64().is_some()),
        "{body}"
    );

    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/search",
            Some(json!({"query": "bulk capability", "k": 999})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["hits"].as_array().unwrap().len(),
        50,
        "k clamps to 50 even with 55 seeded: {body}"
    );
}

/// Target binding guard: serde-valid kinds outside the routable set
/// (project) and blank target names are 400s; rebinding overwrites (the
/// definition row keeps exactly one newest target); an unknown capability
/// id still answers 200 with a null target — the read pins the
/// `{"target": null}` contract instead of a 404.
#[tokio::test]
async fn target_guard_rebind_and_unknown_id_is_null() {
    let h = Harness::new().await;
    let id = seed_cap(&h, "guard target capability").await;
    let path = format!("/api/brain/capabilities/{id}/target");

    let (status, body) = h
        .req(
            Method::PUT,
            &path,
            Some(json!({"kind": "project", "target": "x"})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(
        body["error"],
        json!("capability target must name an agent, team, workflow or operator")
    );
    let (status, body) = h
        .req(
            Method::PUT,
            &path,
            Some(json!({"kind": "agent", "target": "   "})),
        )
        .await;
    assert_eq!(status, 400, "{body}");

    let (status, body) = h
        .req(
            Method::PUT,
            &path,
            Some(json!({"kind": "agent", "target": "act"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["kind"], json!("agent"));
    // Rebinding replaces: the read shows only the newest binding.
    let (status, body) = h
        .req(
            Method::PUT,
            &path,
            Some(json!({"kind": "team", "target": "crew"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["kind"], json!("team"));
    let (status, body) = h.req(Method::GET, &path, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["target"], json!({"kind": "team", "target": "crew"}));

    let (status, body) = h
        .req(
            Method::PUT,
            &path,
            Some(json!({"kind": "operator", "target": "act"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = h.req(Method::GET, &path, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["target"], json!({"kind": "operator", "target": "act"}));

    let (status, body) = h
        .req(Method::GET, "/api/brain/capabilities/cap-none/target", None)
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["target"], json!(null), "unknown id is null, not 404");
}

#[tokio::test]
async fn deleting_capability_clears_orphan_metadata_on_retry() {
    let h = Harness::new().await;
    let id = seed_cap(&h, "retry deletion").await;
    h.state
        .fleet
        .put_definition(
            "capability_target",
            &id,
            &json!({"kind":"agent","target":"act"}),
        )
        .await
        .unwrap();
    h.state.store.delete_brain_capability(&id).await.unwrap();
    let (status, body) = h
        .req(
            Method::DELETE,
            &format!("/api/brain/capabilities/{id}"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert!(h
        .state
        .fleet
        .definition("capability_target", &id)
        .await
        .unwrap()
        .is_none());
}
