use super::*;

#[tokio::test]
async fn deleting_saved_plan_removes_versions_and_catalog_entry() {
    let h = Harness::with_brain_kind().await;
    advertise_v4(&h);
    let saved = json!({
        "id":"plan-delete-me",
        "version":1,
        "plan":plan(),
        "changelog":"cleanup test plan",
        "created_at":1
    });
    let (status, body) = h
        .req(Method::POST, "/api/brain/plan-defs", Some(saved))
        .await;
    assert_eq!(status, 200, "{body}");

    let (_, library) = h.req(Method::GET, "/api/brain/library", None).await;
    assert!(library["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .any(|cap| cap["id"] == "plan-plan-delete-me@1"));

    let (status, body) = h
        .req(Method::DELETE, "/api/brain/plan-defs/plan-delete-me", None)
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["deleted"], "plan-delete-me");

    let (_, plans) = h.req(Method::GET, "/api/brain/plan-defs", None).await;
    assert!(!plans["plans"]
        .as_array()
        .unwrap()
        .iter()
        .any(|plan| plan["id"] == "plan-delete-me"));
    let (status, _) = h
        .req(
            Method::GET,
            "/api/brain/plan-defs/plan-delete-me/versions/1",
            None,
        )
        .await;
    assert_eq!(status, 404);
    let (_, library) = h.req(Method::GET, "/api/brain/library", None).await;
    assert!(!library["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .any(|cap| cap["id"] == "plan-plan-delete-me@1"));

    let (status, _) = h
        .req(Method::DELETE, "/api/brain/plan-defs/plan-delete-me", None)
        .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn deleting_saved_plan_is_blocked_while_a_run_uses_it() {
    let h = Harness::with_brain_kind().await;
    advertise_v4(&h);
    let saved = json!({
        "id":"plan-in-use",
        "version":1,
        "plan":plan(),
        "changelog":"active plan",
        "created_at":1
    });
    let (status, body) = h
        .req(Method::POST, "/api/brain/plan-defs", Some(saved))
        .await;
    assert_eq!(status, 200, "{body}");

    let request = json!({
        "id":"brain-plan-in-use",
        "schema_version":7,
        "plan":{"id":"plan-in-use","version":1},
        "inputs":{}
    });
    let (status, body) = h.req(Method::POST, "/api/brain/runs", Some(request)).await;
    assert_eq!(status, 202, "{body}");

    let (status, body) = h
        .req(Method::DELETE, "/api/brain/plan-defs/plan-in-use", None)
        .await;
    assert_eq!(status, 409, "{body}");
    assert!(body["error"]
        .as_str()
        .unwrap()
        .contains("brain plan is referenced by"));
}

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

#[tokio::test]
async fn deleting_plan_rejects_saved_parent_and_unlaunched_nested_run() {
    let h = Harness::with_brain_kind().await;
    advertise_v4(&h);
    let save = |id: &str, plan: Value| json!({"id":id,"version":1,"plan":plan,"changelog":"reference test","created_at":1});
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/plan-defs",
            Some(save("child", plan())),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let mut parent = plan();
    parent["nodes"][0]["capability_id"] = json!("plan-child@1");
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/plan-defs",
            Some(save("parent", parent.clone())),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = h
        .req(Method::DELETE, "/api/brain/plan-defs/child", None)
        .await;
    assert_eq!(status, 409, "{body}");
    assert!(body["error"].as_str().unwrap().contains("plan:parent@1"));
    assert_eq!(
        h.req(Method::DELETE, "/api/brain/plan-defs/parent", None)
            .await
            .0,
        200
    );
    let mut request = request();
    request["plan"] = parent;
    let (status, body) = h.req(Method::POST, "/api/brain/runs", Some(request)).await;
    assert_eq!(status, 202, "{body}");
    let (status, body) = h
        .req(Method::DELETE, "/api/brain/plan-defs/child", None)
        .await;
    assert_eq!(status, 409, "{body}");
    assert!(body["error"].as_str().unwrap().contains(RUN));
    h.report_status(RUN, ExecutionStatus::Done).await;
    let (status, body) = h
        .req(Method::DELETE, "/api/brain/plan-defs/child", None)
        .await;
    assert_eq!(status, 200, "{body}");
    assert!(h.state.fleet.assignment(RUN).await.unwrap().is_some());
}
