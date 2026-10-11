//! Tag table writes use the same ownership contract on the Server router.
use crate::support::Harness;
use reqwest::Method;
use serde_json::{json, Value};

async fn post(h: &Harness, path: &str, body: Value) -> Value {
    let (status, response) = h.req(Method::POST, path, Some(body)).await;
    assert_eq!(status, 200, "{response}");
    response
}

#[tokio::test]
async fn global_tag_scope_moves_and_fallback_work_on_server() {
    let h = Harness::new().await;
    let global = post(
        &h,
        "/api/project/tags",
        json!({"name":"通用","scope_type":"global"}),
    )
    .await;
    assert_eq!(global["scope_id"], "");
    let initiative = post(&h, "/api/project/initiatives", json!({"title":"独立专项"})).await;
    post(
        &h,
        "/api/project/todos",
        json!({"title":"未归属","draft":"","tag_ids":[global["id"]]}),
    )
    .await;
    post(&h, "/api/project/todos", json!({"title":"专项任务","draft":"","initiative_id":initiative["id"],"tag_ids":[global["id"]]})).await;
    let local = post(
        &h,
        "/api/project/tags",
        json!({"name":"通用","scope_type":"initiative","scope_id":initiative["id"]}),
    )
    .await;
    let path = format!("/api/project/tags/{}", local["id"].as_str().unwrap());
    let (status, _) = h
        .req(
            Method::PATCH,
            &path,
            Some(json!({"name":"通用","scope_type":"global","scope_id":""})),
        )
        .await;
    assert_eq!(status, 409);
    let (status, _) = h.req(Method::DELETE, &path, None).await;
    assert_eq!(status, 200);
    let (_, todos) = h.req(Method::GET, "/api/project/todos", None).await;
    for todo in todos["todos"].as_array().unwrap() {
        assert_eq!(todo["tag_ids"], json!([global["id"]]));
    }
    let path = format!("/api/project/tags/{}", global["id"].as_str().unwrap());
    let (status, _) = h
        .req(
            Method::PATCH,
            &path,
            Some(json!({"name":"通用","scope_type":"initiative","scope_id":initiative["id"]})),
        )
        .await;
    assert_eq!(status, 200);
    let (_, todos) = h.req(Method::GET, "/api/project/todos", None).await;
    let unassigned = todos["todos"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["title"] == "未归属")
        .unwrap();
    assert_eq!(unassigned["tag_ids"], json!([]));
    let (status, moved) = h
        .req(
            Method::PATCH,
            &path,
            Some(json!({"name":"通用","scope_type":"global"})),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(moved["id"], global["id"]);
    assert_eq!(moved["scope_id"], "");
}

#[tokio::test]
async fn editable_tag_scope_works_on_server_and_rejects_conflicting_move() {
    let h = Harness::new().await;
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
    post(&h, "/api/project/todos", json!({"title":"任务","draft":"说明","initiative_id":initiative["id"],"tag_ids":[tag["id"]]})).await;
    let path = format!("/api/project/tags/{}", tag["id"].as_str().unwrap());
    let (status, moved) = h
        .req(
            Method::PATCH,
            &path,
            Some(json!({"name":"模块","scope_type":"initiative","scope_id":initiative["id"]})),
        )
        .await;
    assert_eq!(status, 200, "{moved}");
    assert_eq!(moved["id"], tag["id"]);
    assert_eq!(moved["scope_type"], "initiative");
    let (_, todos) = h.req(Method::GET, "/api/project/todos", None).await;
    assert_eq!(todos["todos"][0]["tag_ids"], json!([tag["id"]]));
    let parent = post(
        &h,
        "/api/project/tags",
        json!({"name":"模块","scope_type":"project","scope_id":project["id"]}),
    )
    .await;
    let (_, before) = h.req(Method::GET, "/api/project/overview", None).await;
    let (status, _) = h
        .req(
            Method::PATCH,
            &path,
            Some(json!({"name":"模块","scope_type":"project","scope_id":project["id"]})),
        )
        .await;
    assert_eq!(status, 409);
    let (_, after) = h.req(Method::GET, "/api/project/overview", None).await;
    assert_eq!(after, before);
    let (status, _) = h.req(Method::DELETE, &path, None).await;
    assert_eq!(status, 200);
    let (_, todos) = h.req(Method::GET, "/api/project/todos", None).await;
    assert_eq!(todos["todos"][0]["tag_ids"], json!([parent["id"]]));
}
