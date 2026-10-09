//! Tag and hierarchy contracts through the actual authenticated HTTP router.
mod support;
use axum::http::StatusCode;
use serde_json::json;
use support::project_app::{call, harness};
#[tokio::test]
async fn tags_validate_scope_override_names_and_round_trip_in_overview() {
    let h = harness().await;
    let (_, project) = call(
        &h.app,
        "POST",
        "/api/project/goals",
        Some(json!({"title":"项目"})),
    )
    .await;
    let (_, initiative) = call(
        &h.app,
        "POST",
        "/api/project/initiatives",
        Some(json!({"title":"专项","goal_id":project["id"]})),
    )
    .await;
    let (status, parent_tag) = call(
        &h.app,
        "POST",
        "/api/project/tags",
        Some(json!({"scope_type":"project","scope_id":project["id"],"name":" 前端 "})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(parent_tag["name"], "前端");
    let (_, todo) = call(&h.app, "POST", "/api/project/todos", Some(json!({"title":"任务","draft":"说明","initiative_id":initiative["id"],"tag_ids":[parent_tag["id"]]}))).await;
    let (status, local_tag) = call(
        &h.app,
        "POST",
        "/api/project/tags",
        Some(json!({"scope_type":"initiative","scope_id":initiative["id"],"name":"前端"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, overview) = call(&h.app, "GET", "/api/project/overview", None).await;
    assert_eq!(
        overview["goals"][0]["initiatives"][0]["todos"][0]["tag_ids"],
        json!([local_tag["id"]])
    );
    let (status, _) = call(
        &h.app,
        "POST",
        "/api/project/tags",
        Some(json!({"scope_type":"initiative","scope_id":initiative["id"],"name":"前端"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = call(
        &h.app,
        "PATCH",
        &format!("/api/project/todos/{}", todo["id"].as_str().unwrap()),
        Some(json!({"initiative_id":null,"tag_ids":[local_tag["id"]]})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = call(&h.app, "GET", "/api/project/milestones", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
#[tokio::test]
async fn scoped_reorder_rejects_foreign_cards_and_progress_uses_board_status() {
    let h = harness().await;
    let (_, i) = call(
        &h.app,
        "POST",
        "/api/project/initiatives",
        Some(json!({"title":"专项"})),
    )
    .await;
    let (_, t) = call(
        &h.app,
        "POST",
        "/api/project/todos",
        Some(json!({"title":"任务","draft":"说明","initiative_id":i["id"],"board_status":"done"})),
    )
    .await;
    let (_, outside) = call(
        &h.app,
        "POST",
        "/api/project/todos",
        Some(json!({"title":"未归属","draft":""})),
    )
    .await;
    let (status, _) = call(
        &h.app,
        "PUT",
        "/api/project/todos/order",
        Some(json!({"initiative_id":i["id"],"board_status":"done","ids":[t["id"],outside["id"]]})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (_, overview) = call(&h.app, "GET", "/api/project/overview", None).await;
    assert_eq!(
        overview["standalone_initiatives"][0]["progress"],
        json!({"total":1,"done":1})
    );
    assert_eq!(overview["backlog"][0]["board_status"], "backlog");
}

#[tokio::test]
async fn tag_list_rename_and_delete_validate_scope_and_missing_records() {
    let h = harness().await;
    let (_, initiative) = call(
        &h.app,
        "POST",
        "/api/project/initiatives",
        Some(json!({"title":"独立专项"})),
    )
    .await;
    let id = initiative["id"].as_str().unwrap();
    let (status, _) = call(
        &h.app,
        "POST",
        "/api/project/tags",
        Some(json!({"name":"x","scope_type":"initiative","scope_id":"missing"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, tag) = call(
        &h.app,
        "POST",
        "/api/project/tags",
        Some(json!({"name":"模块","scope_type":"initiative","scope_id":id})),
    )
    .await;
    let path = format!("/api/project/tags/{}", tag["id"].as_str().unwrap());
    let (status, _) = call(
        &h.app,
        "PATCH",
        &path,
        Some(json!({"name":"  ","scope_type":"initiative","scope_id":id})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = call(
        &h.app,
        "PATCH",
        &path,
        Some(json!({"name":" 改名 ","scope_type":"initiative","scope_id":id})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, listed) = call(
        &h.app,
        "GET",
        &format!("/api/project/tags?scope_type=initiative&scope_id={id}"),
        None,
    )
    .await;
    assert_eq!(listed["tags"][0]["name"], "改名");
    let (_, empty) = call(
        &h.app,
        "GET",
        "/api/project/tags?scope_type=project&scope_id=missing",
        None,
    )
    .await;
    assert!(empty["tags"].as_array().unwrap().is_empty());
    let (status, _) = call(&h.app, "DELETE", &path, None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(&h.app, "DELETE", &path, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = call(
        &h.app,
        "PATCH",
        &path,
        Some(json!({"name":"x","scope_type":"initiative","scope_id":id})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
