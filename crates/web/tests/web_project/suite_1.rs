use super::*;

#[tokio::test]
async fn todo_board_reorder_moves_cards_and_rejects_missing_ids() {
    let h = harness().await;
    let mut ids = Vec::new();
    for title in ["first", "second"] {
        let (status, todo) = call(
            &h.app,
            "POST",
            "/api/project/todos",
            Some(json!({"title":title,"draft":"work"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{todo}");
        ids.push(todo["id"].as_str().unwrap().to_owned());
    }
    let (status, body) = call(
        &h.app,
        "PUT",
        "/api/project/todos/order",
        Some(json!({"board_status":"in_progress","ids":[ids[1],ids[0]]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (_, listed) = call(&h.app, "GET", "/api/project/todos", None).await;
    assert_eq!(todo_row(&listed, &ids[1])["board_status"], "in_progress");
    assert!(
        todo_row(&listed, &ids[1])["position"].as_i64().unwrap()
            < todo_row(&listed, &ids[0])["position"].as_i64().unwrap()
    );

    let (status, _) = call(
        &h.app,
        "PUT",
        "/api/project/todos/order",
        Some(json!({"board_status":"done","ids":[ids[0],"missing"]})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (_, listed) = call(&h.app, "GET", "/api/project/todos", None).await;
    assert_eq!(todo_row(&listed, &ids[0])["board_status"], "in_progress");
    let (status, body) = call(
        &h.app,
        "PATCH",
        &format!("/api/project/todos/{}", ids[0]),
        Some(json!({"position": -1})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, body) = call(
        &h.app,
        "PATCH",
        &format!("/api/project/todos/{}", ids[0]),
        Some(json!({"position": 0})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn todo_draft_does_not_dispatch_mentions() {
    let h = harness().await;
    let (status, todo) = call(
        &h.app,
        "POST",
        "/api/project/todos",
        Some(json!({"title":"bound","draft":"work\n@operator"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{todo}");
    assert_eq!(todo["capability_id"], Value::Null);
    let id = todo["id"].as_str().unwrap();
    let (status, _) = call(
        &h.app,
        "PATCH",
        &format!("/api/project/todos/{id}"),
        Some(json!({"draft":"ordinary @operator mention"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, listed) = call(&h.app, "GET", "/api/project/todos", None).await;
    assert_eq!(todo_row(&listed, id)["capability_id"], Value::Null);
}

#[tokio::test]
async fn goal_initiative_todo_crud_contract() {
    let h = harness().await;

    // Goal create → server id, active status, trimmed title.
    let (status, goal) = call(
        &h.app,
        "POST",
        "/api/project/goals",
        Some(json!({ "title": "  目标A  ", "detail_md": "初始" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{goal}");
    let gid = goal["id"].as_str().unwrap().to_string();
    assert!(gid.starts_with("pg-"), "id: {gid}");
    assert_eq!(goal["status"], "active");
    assert_eq!(goal["title"], "目标A");

    // Patch title + detail; list reflects it.
    let (status, v) = call(
        &h.app,
        "PATCH",
        &format!("/api/project/goals/{gid}"),
        Some(json!({ "title": "目标A2", "detail_md": "改后" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let (_, goals) = call(&h.app, "GET", "/api/project/goals", None).await;
    assert_eq!(goals["goals"].as_array().unwrap().len(), 1);
    assert_eq!(goals["goals"][0]["title"], "目标A2");
    assert_eq!(goals["goals"][0]["detail_md"], "改后");

    // Unknown goal_id is a 404 with the shared error body.
    let (status, v) = call(
        &h.app,
        "POST",
        "/api/project/initiatives",
        Some(json!({ "goal_id": "pg-bogus", "title": "x" })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{v}");
    assert_eq!(v["ok"], false);
    assert!(v["error"].as_str().unwrap().contains("pg-bogus"));

    // Initiative create + status patch + goal filter.
    let (status, ms) = call(
        &h.app,
        "POST",
        "/api/project/initiatives",
        Some(json!({ "goal_id": gid, "title": "专项1" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{ms}");
    let mid = ms["id"].as_str().unwrap().to_string();
    assert!(mid.starts_with("pi-"));
    let (status, v) = call(
        &h.app,
        "PATCH",
        &format!("/api/project/initiatives/{mid}"),
        Some(json!({ "status": "in_progress" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let (_, list) = call(
        &h.app,
        "GET",
        &format!("/api/project/initiatives?goal_id={gid}"),
        None,
    )
    .await;
    assert_eq!(list["initiatives"].as_array().unwrap().len(), 1);
    assert_eq!(list["initiatives"][0]["status"], "in_progress");
    let (_, unfiltered) = call(&h.app, "GET", "/api/project/initiatives", None).await;
    assert_eq!(unfiltered["initiatives"].as_array().unwrap().len(), 1);
    let (_, other) = call(
        &h.app,
        "GET",
        "/api/project/initiatives?goal_id=pg-none",
        None,
    )
    .await;
    assert_eq!(other["initiatives"].as_array().unwrap().len(), 0);

    // Todo under the initiative; JSON null initiative_id clears to backlog.
    let (status, todo) = call(
        &h.app,
        "POST",
        "/api/project/todos",
        Some(json!({ "initiative_id": mid, "title": "待办1", "draft": "草稿" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{todo}");
    let tid = todo["id"].as_str().unwrap().to_string();
    assert!(tid.starts_with("pt-"));
    assert_eq!(todo["status"], "draft");
    assert_eq!(todo["agent"], "act", "default agent");
    let (status, v) = call(
        &h.app,
        "PATCH",
        &format!("/api/project/todos/{tid}"),
        Some(json!({ "initiative_id": null })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let (_, todos) = call(&h.app, "GET", "/api/project/todos", None).await;
    assert_eq!(todo_row(&todos, &tid)["initiative_id"], Value::Null);

    // Missing required field (draft) is an axum Json rejection → 4xx.
    let (status, v) = call(
        &h.app,
        "POST",
        "/api/project/todos",
        Some(json!({ "title": "无草稿" })),
    )
    .await;
    assert!(
        status.is_client_error(),
        "missing required field must 4xx, got {status}: {v}"
    );

    // Unknown ids on patch/delete are 404s.
    let (status, v) = call(
        &h.app,
        "PATCH",
        "/api/project/todos/pt-none",
        Some(json!({ "title": "x" })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{v}");

    // Cascades: todo → initiative → goal empties every list.
    for uri in [
        format!("/api/project/todos/{tid}"),
        format!("/api/project/initiatives/{mid}"),
        format!("/api/project/goals/{gid}"),
    ] {
        let (status, v) = call(&h.app, "DELETE", &uri, None).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {v}");
        assert_eq!(v["deleted"], true);
    }
    let (_, todos) = call(&h.app, "GET", "/api/project/todos", None).await;
    assert_eq!(todos["todos"].as_array().unwrap().len(), 0);
    let (_, goals) = call(&h.app, "GET", "/api/project/goals", None).await;
    assert_eq!(goals["goals"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn todo_executor_fields_create_patch_and_run_shape() {
    let h = harness().await;

    // team todo with ref + valid inline spec → round-trips all three.
    let team_spec = serde_json::json!({
        "name": "crew",
        "captain": { "node_id": "act", "name": "队长" },
        "members": [{ "node_id": "explore", "name": "侦察" }]
    })
    .to_string();
    let (status, todo) = call(
        &h.app,
        "POST",
        "/api/project/todos",
        Some(json!({
            "title": "团队活",
            "draft": "多人协作",
            "executor_kind": "team",
            "executor_ref": "  crew-x  ",
            "executor_spec": team_spec,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{todo}");
    let tid = todo["id"].as_str().unwrap().to_string();
    assert_eq!(todo["executor_kind"], "team");
    assert_eq!(todo["executor_ref"], "crew-x", "ref is trimmed");
    assert!(todo["executor_spec"]
        .as_str()
        .unwrap()
        .contains("\"captain\""));
    let (_, list) = call(&h.app, "GET", "/api/project/todos", None).await;
    let row = todo_row(&list, &tid);
    assert_eq!(row["executor_kind"], "team");
    assert_eq!(row["executor_ref"], "crew-x");

    // Unknown kind string → 400 naming it.
    let (status, v) = call(
        &h.app,
        "POST",
        "/api/project/todos",
        Some(json!({ "title": "x", "draft": "y", "executor_kind": "nope" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"]
        .as_str()
        .unwrap()
        .contains("unknown executor_kind: nope"));

    // dag + invalid DagSpec JSON → 400 mentioning the spec problem.
    let (status, v) = call(
        &h.app,
        "POST",
        "/api/project/todos",
        Some(json!({
            "title": "x", "draft": "y", "executor_kind": "dag",
            "executor_spec": "{\"name\":\"d\",\"steps\":[{\"name\":\"s\",\"kind\":{\"type\":\"agent\"}}]}"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"].as_str().unwrap().contains("executor_spec"));

    // agent + spec → 400 (agent takes no spec).
    let (status, v) = call(
        &h.app,
        "POST",
        "/api/project/todos",
        Some(json!({ "title": "x", "draft": "y", "executor_spec": "{}" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"]
        .as_str()
        .unwrap()
        .contains("agent executor takes no spec"));

    // PATCH: null-clear executor_ref (double option) + swap kind; the spec
    // survives the swap only when it validates for the new kind — swapping
    // to dag with a team spec must 400, so clear the spec in the same patch.
    let (status, v) = call(
        &h.app,
        "PATCH",
        &format!("/api/project/todos/{tid}"),
        Some(json!({ "executor_ref": null, "executor_spec": null })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let (status, v) = call(
        &h.app,
        "PATCH",
        &format!("/api/project/todos/{tid}"),
        Some(json!({ "executor_kind": "dag" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let (_, list) = call(&h.app, "GET", "/api/project/todos", None).await;
    let row = todo_row(&list, &tid);
    assert_eq!(row["executor_kind"], "dag");
    assert_eq!(row["executor_ref"], Value::Null, "null cleared the ref");
    assert_eq!(row["executor_spec"], Value::Null, "null cleared the spec");

    // Spec-only patch against the CURRENT kind: dag + a broken spec 400s
    // even without executor_kind in the body.
    let (status, v) = call(
        &h.app,
        "PATCH",
        &format!("/api/project/todos/{tid}"),
        Some(json!({ "executor_spec": "not json" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"].as_str().unwrap().contains("executor_spec"));

    // Run rows carry the resolved executor_kind — a plan run is agent even
    // on a dag todo (planning is executor-agnostic).
    h.mock.queue_script(done("# 计划\n1. x"));
    let (status, v) = call(
        &h.app,
        "POST",
        &format!("/api/project/todos/{tid}/plan"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{v}");
    let planned = support::project_app::wait_until(
        &h.app,
        &format!("/api/project/todos/{tid}/runs"),
        "run lands with executor_kind",
        |b| {
            b["runs"].as_array().is_some_and(|runs| {
                runs.iter()
                    .any(|r| r["executor_kind"] == "agent" && r["kind"] == "plan")
            })
        },
    )
    .await;
    let run = planned["runs"][0].clone();
    assert_eq!(run["executor_kind"], "agent");
    assert_eq!(run["kind"], "plan");
}
