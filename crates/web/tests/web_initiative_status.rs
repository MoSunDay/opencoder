mod support;
use axum::http::StatusCode;
use serde_json::json;
use support::project_app::{call, harness};

#[tokio::test]
async fn initiatives_reject_status_and_keep_people_summary_and_todo_board_writes() {
    let h = harness().await;
    let (status, created) = call(
        &h.app,
        "POST",
        "/api/project/initiatives",
        Some(json!({
            "title":"Direction","current_summary":"Progress","owner_user_name":null,
            "follower_user_names":[]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    assert!(created.get("status").is_none());
    let path = format!(
        "/api/project/initiatives/{}",
        created["id"].as_str().unwrap()
    );
    for obsolete in ["planned", "in_progress", "done", "", "active"] {
        let (status, _) = call(
            &h.app,
            "POST",
            "/api/project/initiatives",
            Some(json!({"title":"Rejected","status":obsolete})),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let (status, _) = call(
            &h.app,
            "PATCH",
            &path,
            Some(json!({"title":"Must not be written","status":obsolete})),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    }
    let (status, result) = call(
        &h.app,
        "PATCH",
        &path,
        Some(json!({
            "title":"Renamed","sort":4,"owner_user_name":null,"follower_user_names":[]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    let (_, list) = call(&h.app, "GET", "/api/project/initiatives", None).await;
    let records = list["initiatives"].as_array().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["title"], "Renamed");
    assert_eq!(records[0]["current_summary"], "Progress");
    assert!(records[0].get("status").is_none());
    let (status, todo) = call(
        &h.app,
        "POST",
        "/api/project/todos",
        Some(json!({
            "title":"Task","draft":"Work","initiative_id":created["id"],"board_status":"todo"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{todo}");
    let (status, result) = call(
        &h.app,
        "PATCH",
        &format!("/api/project/todos/{}", todo["id"].as_str().unwrap()),
        Some(json!({"board_status":"done"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    let (_, overview) = call(&h.app, "GET", "/api/project/overview", None).await;
    assert!(overview["standalone_initiatives"][0]
        .get("status")
        .is_none());
    assert_eq!(overview["standalone_initiatives"][0]["progress"]["done"], 1);
}
