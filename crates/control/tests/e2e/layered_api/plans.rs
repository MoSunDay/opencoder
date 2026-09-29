use super::*;

#[tokio::test]
async fn saved_plan_versions_are_capabilities_and_nested_runs_pin_the_version() {
    let h = Harness::with_brain_kind().await;
    advertise_v4(&h);
    let mut saved =
        json!({"id":"plan-child","version":1,"plan":plan(),"changelog":"initial","created_at":1});
    saved["plan"]["transitions"] =
        json!([{"from":"scan-layer","to":"apply-layer","condition":"obsolete condition"}]);
    let (status, body) = h
        .req(Method::POST, "/api/brain/plan-defs", Some(saved.clone()))
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["version"]["plan"]["transitions"],
        saved["plan"]["transitions"]
    );
    let (status, retried) = h
        .req(Method::POST, "/api/brain/plan-defs", Some(saved.clone()))
        .await;
    assert_eq!(status, 200, "{retried}");
    let (status, body) = h.req(Method::GET, "/api/brain/library", None).await;
    assert_eq!(status, 200, "{body}");
    let cap = body["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "plan-plan-child@1")
        .unwrap();
    assert_eq!(cap["kind"], "brain");
    assert_eq!(cap["definition"]["version"], 1);
    let mut request = request();
    request["plan"]["nodes"] = json!([{"node_id":"nested","title":"完成子计划","capability_id":"plan-plan-child@1","layer_id":"scan-layer","objective":"finish child"}]);
    request["plan"]["layers"]
        .as_array_mut()
        .unwrap()
        .truncate(1);
    request["plan"]["transitions"] = json!([]);
    request["plan"]["edges"] = json!([]);
    let (status, body) = h.req(Method::POST, "/api/brain/runs", Some(request)).await;
    assert_eq!(status, 202, "{body}");
    let assignment = h.state.fleet.assignment(RUN).await.unwrap().unwrap();
    assert_eq!(
        assignment.request.input["layered_request"]["plan"]["transitions"],
        json!([{"from":"scan-layer","to":"scan-layer","condition":"Evidence requires rework in this previously executed milestone"}])
    );
    assert_eq!(
        assignment.request.input["capability_scope"][0]["version"],
        "1"
    );
    let mut next = saved;
    next["version"] = json!(2);
    next["plan"]["title"] = json!("updated child");
    let (status, body) = h
        .req(Method::POST, "/api/brain/plan-defs", Some(next))
        .await;
    assert_eq!(status, 200, "{body}");
    let (_, body) = h.req(Method::GET, "/api/brain/library", None).await;
    let caps = body["capabilities"].as_array().unwrap();
    assert!(caps.iter().any(|c| c["id"] == "plan-plan-child@1"));
    assert!(caps.iter().any(|c| c["id"] == "plan-plan-child@2"));
}

#[tokio::test]
async fn new_plan_save_is_idempotent_and_keeps_paths_for_retained_runtimes() {
    let h = Harness::with_brain_kind().await;
    advertise_v4(&h);
    let submitted = json!({"id":"plan-autonomous","version":1,"plan":plan(),
        "changelog":"initial","created_at":1});
    let (status, first) = h
        .req(
            Method::POST,
            "/api/brain/plan-defs",
            Some(submitted.clone()),
        )
        .await;
    assert_eq!(status, 200, "{first}");
    let paths = first["version"]["plan"]["transitions"].as_array().unwrap();
    assert_eq!(paths.len(), 4);
    assert!(paths
        .iter()
        .any(|edge| edge["from"] == "scan-layer" && edge["to"] == "apply-layer"));
    assert!(paths
        .iter()
        .any(|edge| edge["from"] == "apply-layer" && edge["to"] == "scan-layer"));
    let (status, retried) = h
        .req(Method::POST, "/api/brain/plan-defs", Some(submitted))
        .await;
    assert_eq!(status, 200, "{retried}");
    assert_eq!(retried["version"], first["version"]);
}

#[tokio::test]
async fn old_plan_writes_are_rejected_without_persisting_a_version() {
    let h = Harness::with_brain_kind().await;
    for version in [1, 2, 3, 4] {
        let body = json!({"id":"plan-old","version":1,"plan":{"schema_version":version,"title":"old","objective":"old","capability_ids":["builtin-agent-act"]},"changelog":"old","created_at":1});
        let (status, _) = h
            .req(Method::POST, "/api/brain/plan-defs", Some(body))
            .await;
        assert_eq!(status, 400);
    }
    assert!(h
        .state
        .fleet
        .brain_plan_document("plan-old", 1)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn inline_plan_defaults_and_run_overrides_are_frozen_for_dispatch() {
    let h = Harness::with_brain_kind().await;
    advertise_v4(&h);
    let mut body = request();
    body["plan"]["inputs"] = json!({"repo":"default-repo","branch":"main"});
    body["inputs"] = json!({"branch":"review"});
    let (status, receipt) = h.req(Method::POST, "/api/brain/runs", Some(body)).await;
    assert_eq!(status, 202, "{receipt}");
    let assignment = h.state.fleet.assignment(RUN).await.unwrap().unwrap();
    assert_eq!(
        assignment.request.input["layered_request"]["inputs"],
        json!({"repo":"default-repo","branch":"review"})
    );
}
