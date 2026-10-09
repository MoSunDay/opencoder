//! Editable Tag ownership through the authenticated Web router.
mod support;
use axum::http::StatusCode;
use serde_json::{json, Value};
use support::project_app::{call, harness, Harness};

async fn post(h: &Harness, path: &str, body: Value) -> Value {
    let (status, result) = call(&h.app, "POST", path, Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    result
}

#[tokio::test]
async fn update_moves_tag_without_recreating_it_and_reconciles_todos() {
    let h = harness().await;
    let project = post(&h, "/api/project/goals", json!({"title":"项目"})).await;
    let source = post(
        &h,
        "/api/project/initiatives",
        json!({"title":"源专项","goal_id":project["id"]}),
    )
    .await;
    let target = post(&h, "/api/project/initiatives", json!({"title":"独立专项"})).await;
    let parent = post(
        &h,
        "/api/project/tags",
        json!({"name":"模块","scope_type":"project","scope_id":project["id"]}),
    )
    .await;
    let local = post(
        &h,
        "/api/project/tags",
        json!({"name":"模块","scope_type":"initiative","scope_id":source["id"]}),
    )
    .await;
    let todo = post(
        &h,
        "/api/project/todos",
        json!({"title":"任务","draft":"说明","initiative_id":source["id"],"tag_ids":[local["id"]]}),
    )
    .await;
    let path = format!("/api/project/tags/{}", local["id"].as_str().unwrap());
    let (status, moved) = call(
        &h.app,
        "PATCH",
        &path,
        Some(json!({"name":" 模块 ","scope_type":"initiative","scope_id":target["id"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(moved["id"], local["id"]);
    assert_eq!(moved["name"], "模块");
    assert_eq!(moved["scope_id"], target["id"]);
    let (_, todos) = call(&h.app, "GET", "/api/project/todos", None).await;
    assert_eq!(todos["todos"][0]["id"], todo["id"]);
    assert_eq!(todos["todos"][0]["tag_ids"], json!([parent["id"]]));
    let (status, changed) = call(
        &h.app,
        "PATCH",
        &path,
        Some(json!({"name":" 项目模块 ","scope_type":"project","scope_id":project["id"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(changed["scope_type"], "project");
    assert_eq!(changed["id"], local["id"]);
}

#[tokio::test]
async fn invalid_update_keeps_catalog_and_todo_links_unchanged() {
    let h = harness().await;
    let project = post(&h, "/api/project/goals", json!({"title":"项目"})).await;
    let initiative = post(
        &h,
        "/api/project/initiatives",
        json!({"title":"专项","goal_id":project["id"]}),
    )
    .await;
    let tag = post(
        &h,
        "/api/project/tags",
        json!({"name":"模块","scope_type":"project","scope_id":project["id"]}),
    )
    .await;
    post(
        &h,
        "/api/project/tags",
        json!({"name":"重复","scope_type":"initiative","scope_id":initiative["id"]}),
    )
    .await;
    post(&h, "/api/project/todos", json!({"title":"任务","draft":"说明","initiative_id":initiative["id"],"tag_ids":[tag["id"]]})).await;
    let (_, before) = call(&h.app, "GET", "/api/project/overview", None).await;
    let path = format!("/api/project/tags/{}", tag["id"].as_str().unwrap());
    for (body, expected) in [
        (
            json!({"name":"重复","scope_type":"initiative","scope_id":initiative["id"]}),
            StatusCode::CONFLICT,
        ),
        (
            json!({"name":"模块","scope_type":"initiative","scope_id":"missing"}),
            StatusCode::NOT_FOUND,
        ),
        (
            json!({"name":"模块","scope_type":"todo","scope_id":initiative["id"]}),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({"name":"  ","scope_type":"project","scope_id":project["id"]}),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({"name":"长".repeat(129),"scope_type":"project","scope_id":project["id"]}),
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let (status, response) = call(&h.app, "PATCH", &path, Some(body)).await;
        assert_eq!(status, expected, "{response}");
        let (_, after) = call(&h.app, "GET", "/api/project/overview", None).await;
        assert_eq!(after, before);
    }
    let (status, _) = call(
        &h.app,
        "PATCH",
        "/api/project/tags/missing",
        Some(json!({"name":"模块","scope_type":"project","scope_id":project["id"]})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
