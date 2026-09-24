use super::*;

#[tokio::test]
async fn pc_plan_install_is_idempotent_and_requires_original_problem() {
    let h = Harness::with_brain_kind().await;
    advertise_v4(&h);
    let (status, first) = h
        .req(Method::POST, "/api/brain/pc-issue/plan", Some(json!({})))
        .await;
    assert_eq!(status, 200, "{first}");
    assert_eq!(first["definition"]["id"], "pc-issue");
    let (status, second) = h
        .req(Method::POST, "/api/brain/pc-issue/plan", Some(json!({})))
        .await;
    assert_eq!(status, 200, "{second}");
    assert_eq!(first["definition"], second["definition"]);
    let (status,body)=h.req(Method::POST,"/api/brain/runs",Some(json!({
        "schema_version":6,"id":"brain-pc-empty","plan":{"id":"pc-issue","version":1},"inputs":{}
    }))).await;
    assert_eq!(status, 400, "{body}");
    assert!(body["error"].as_str().unwrap().contains("problem.text"));
}

#[tokio::test]
async fn pc_plan_install_appends_schema_six_after_legacy_version() {
    let h = Harness::with_brain_kind().await;
    advertise_v4(&h);
    h.state
        .fleet
        .put_definition(
            "dag",
            "uicase-regression",
            &json!({
                "id":"uicase-regression","name":"uicase-regression","spec":{}
            }),
        )
        .await
        .unwrap();
    let (status, library) = h.req(Method::GET, "/api/brain/library", None).await;
    assert_eq!(status, 200, "{library}");
    let ui = library["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|cap| cap["id"] == "dag-uicase-regression")
        .unwrap();
    assert!(ui["definition"].is_null());
    assert!(ui["unavailable_reason"]
        .to_string()
        .contains("device_count"));
    let mut unavailable_plan = plan();
    unavailable_plan["nodes"][0]["capability_ids"] = json!(["dag-uicase-regression"]);
    let (status, rejection) = h
        .req(
            Method::POST,
            "/api/brain/plan-defs/validate",
            Some(unavailable_plan),
        )
        .await;
    assert_eq!(status, 400, "{rejection}");
    assert!(
        rejection.to_string().contains("device_count"),
        "{rejection}"
    );
    let mut legacy_plan = opencoder_core::brain::pc_issue::plan();
    legacy_plan["schema_version"] = json!(5);
    let legacy: opencoder_core::brain::PlanVersion<serde_json::Value> =
        serde_json::from_value(json!({
            "id":"pc-issue","version":1,"plan":legacy_plan,
            "changelog":"legacy","created_at":1
        }))
        .unwrap();
    h.state
        .fleet
        .save_brain_plan_document(&legacy)
        .await
        .unwrap();

    let (status, installed) = h
        .req(Method::POST, "/api/brain/pc-issue/plan", Some(json!({})))
        .await;
    assert_eq!(status, 200, "{installed}");
    assert_eq!(installed["definition"]["latest_version"], 2);
    assert_eq!(
        h.state
            .fleet
            .brain_plan_document("pc-issue", 1)
            .await
            .unwrap()
            .unwrap(),
        legacy
    );
    assert_eq!(
        h.state
            .fleet
            .brain_plan_document("pc-issue", 2)
            .await
            .unwrap()
            .unwrap()
            .plan["schema_version"],
        6
    );
    let (status, retried) = h
        .req(Method::POST, "/api/brain/pc-issue/plan", Some(json!({})))
        .await;
    assert_eq!(status, 200, "{retried}");
    assert_eq!(installed["definition"], retried["definition"]);
}

#[tokio::test]
async fn problem_attachments_are_immutable_and_tampered_references_rejected() {
    let h = Harness::with_brain_kind().await;
    let (status,reference)=h.req(Method::POST,"/api/brain/attachments",Some(json!({
        "name":"screenshot.png","data_url":"data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC"
    }))).await;
    assert_eq!(status, 200, "{reference}");
    let (status, stored) = h
        .req(
            Method::GET,
            &format!(
                "/api/brain/attachments/{}",
                reference["id"].as_str().unwrap()
            ),
            None,
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(stored["reference"], reference);
    let mut altered = reference;
    altered["sha256"] = json!("0".repeat(64));
    let (status,body)=h.req(Method::POST,"/api/brain/runs",Some(json!({
        "schema_version":6,"id":"brain-pc-altered","plan":opencoder_core::brain::pc_issue::plan(),
        "inputs":{"problem":{"text":"real problem","images":[altered]}}
    }))).await;
    assert_eq!(status, 400, "{body}");
    assert!(body["error"]
        .as_str()
        .unwrap()
        .contains("reference changed"));
    let (status, _) = h
        .req(
            Method::POST,
            "/api/brain/attachments",
            Some(json!({"name":"bad.svg","data_url":"data:image/svg+xml;base64,PHN2Zz4="})),
        )
        .await;
    assert_eq!(status, 400);
}

#[tokio::test]
async fn pc_plan_cannot_skip_baseline_or_reorder_verification() {
    let h = Harness::with_brain_kind().await;
    let mut plan = opencoder_core::brain::pc_issue::plan();
    plan["nodes"].as_array_mut().unwrap().remove(1);
    plan["edges"] = json!([]);
    let (status, body) = h
        .req(Method::POST, "/api/brain/plan-defs/validate", Some(plan))
        .await;
    assert_eq!(status, 400, "{body}");
}

#[tokio::test]
async fn cancelled_pc_stage_cannot_submit_late_device_or_build_work() {
    let h = Harness::with_brain_kind().await;
    advertise_v4(&h);
    create(&h).await;
    let mut current = snapshot("waiting", 4, 8);
    current["operations"] = json!([{
        "run_id":RUN,"operation_id":"pc-verification","node_id":"verify",
        "layer":4,"round":1,"activation":1,"attempt":1,
        "capability_id":"pc-issue-verify","execution_id":"operator-pc-stage",
        "execution_kind":"operator","status":"running","cancel_requested":true,
        "source_sequence":null
    }]);
    h.node.set_brain("snapshot", 200, current);
    for (kind, target) in [("dag", "device-cases"), ("team", "jy-builder")] {
        let (status, body) = h
            .req(
                Method::POST,
                "/api/executions",
                Some(json!({
                    "id":format!("{kind}-pc-layered-e2e-verify-1"),"kind":kind,"target":target,
                    "input":{"device_count":1,"case_source":"/cases.json","case_ids":["original"],"pc_issue_parent":{"execution_id":"operator-pc-stage","run_id":RUN}}
                })),
            )
            .await;
        assert_eq!(status, 409, "{body}");
        assert!(body["error"].as_str().unwrap().contains("PC stage stopped"));
    }
}

#[tokio::test]
async fn pc_device_child_requires_frozen_node_and_deterministic_identity() {
    let h = Harness::with_brain_kind().await;
    advertise_v4(&h);
    let mut root = request();
    root["inputs"] = json!({"settings":{"device_node":"node-e2e","build_node":"node-e2e"}});
    let (status, body) = h.req(Method::POST, "/api/brain/runs", Some(root)).await;
    assert_eq!(status, 202, "{body}");
    let mut current = snapshot("waiting", 2, 8);
    current["operations"] = json!([{
        "run_id":RUN,"operation_id":"pc-reproduction","node_id":"reproduce",
        "layer":2,"round":1,"activation":2,"attempt":1,
        "capability_id":"pc-issue-reproduce","execution_id":"operator-pc-stage",
        "execution_kind":"operator","status":"running","cancel_requested":false,
        "source_sequence":null
    }]);
    h.node.set_brain("snapshot", 200, current);
    let mut child = json!({"id":"dag-pc-layered-e2e-reproduce-1","kind":"dag",
        "target":"device-cases","node_id":"node-other",
        "input":{"device_count":1,"case_source":"/cases.json","case_ids":["original"],
        "pc_issue_parent":{"execution_id":"operator-pc-stage","run_id":RUN}}});
    let (status, body) = h
        .req(Method::POST, "/api/executions", Some(child.clone()))
        .await;
    assert_eq!(status, 409, "{body}");
    assert!(body["error"].as_str().unwrap().contains("node differs"));
    child["node_id"] = json!("node-e2e");
    let (status, body) = h.req(Method::POST, "/api/executions", Some(child)).await;
    assert_eq!(status, 202, "{body}");
}
