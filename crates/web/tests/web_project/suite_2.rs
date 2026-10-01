use super::*;

#[tokio::test]
async fn patch_kind_only_revalidates_stored_spec() {
    let h = harness().await;

    // team todo with a valid team spec on disk.
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
            "title": "换型",
            "draft": "存档 spec 复检",
            "executor_kind": "team",
            "executor_spec": team_spec,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{todo}");
    let tid = todo["id"].as_str().unwrap().to_string();

    // kind-only PATCH (spec NOT cleared): the stored team spec must be
    // revalidated against the new dag kind → 400 naming executor_spec.
    let (status, v) = call(
        &h.app,
        "PATCH",
        &format!("/api/project/todos/{tid}"),
        Some(json!({ "executor_kind": "dag" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"].as_str().unwrap().contains("executor_spec"));

    // The rejected patch applied nothing: the todo keeps team + its spec.
    let (_, list) = call(&h.app, "GET", "/api/project/todos", None).await;
    let row = todo_row(&list, &tid);
    assert_eq!(row["executor_kind"], "team");
    assert!(row["executor_spec"]
        .as_str()
        .unwrap()
        .contains("\"captain\""));

    // Happy variant: clear the spec in the same patch → 200.
    let (status, v) = call(
        &h.app,
        "PATCH",
        &format!("/api/project/todos/{tid}"),
        Some(json!({ "executor_kind": "dag", "executor_spec": null })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let (_, list) = call(&h.app, "GET", "/api/project/todos", None).await;
    let row = todo_row(&list, &tid);
    assert_eq!(row["executor_kind"], "dag");
    assert_eq!(row["executor_spec"], Value::Null, "null cleared the spec");
}

#[tokio::test]
async fn standalone_relations_and_protected_deletion() {
    let h = harness().await;
    let (status, initiative) = call(
        &h.app,
        "POST",
        "/api/project/initiatives",
        Some(json!({"title":"专项"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{initiative}");
    assert!(initiative["goal_id"].is_null());
    let mid = initiative["id"].as_str().unwrap();
    let (_, todo) = call(
        &h.app,
        "POST",
        "/api/project/todos",
        Some(json!({"title":"任务","draft":"正文","initiative_id":mid})),
    )
    .await;
    let tid = todo["id"].as_str().unwrap();
    let (_, overview) = call(&h.app, "GET", "/api/project/overview", None).await;
    assert_eq!(overview["standalone_initiatives"][0]["todos"][0]["id"], tid);
    let (status, _) = call(
        &h.app,
        "DELETE",
        &format!("/api/project/initiatives/{mid}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (_, goal) = call(
        &h.app,
        "POST",
        "/api/project/goals",
        Some(json!({"title":"项目"})),
    )
    .await;
    let gid = goal["id"].as_str().unwrap();
    for body in [
        json!({"goal_id":gid}),
        json!({"goal_id":null}),
        json!({"title":"改名"}),
    ] {
        let (status, body) = call(
            &h.app,
            "PATCH",
            &format!("/api/project/initiatives/{mid}"),
            Some(body),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    let (_, items) = call(&h.app, "GET", "/api/project/initiatives", None).await;
    assert!(items["initiatives"][0]["goal_id"].is_null());
    let (status, _) = call(
        &h.app,
        "PATCH",
        &format!("/api/project/todos/{tid}"),
        Some(json!({"initiative_id":null})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(
        &h.app,
        "DELETE",
        &format!("/api/project/initiatives/{mid}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, todos) = call(&h.app, "GET", "/api/project/todos", None).await;
    assert_eq!(todos["todos"][0]["draft"], "正文");
}
