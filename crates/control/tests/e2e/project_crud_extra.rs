//! Extra CRUD coverage for `/api/project`: goal list order/shape + trimmed
//! create, patch validation and persistence, goal-level cascade, initiative
//! filters and re-parenting, todo defaults and real-field patches (the base
//! suite's todo patch only exercises a no-op field).

use reqwest::Method;
use serde_json::{json, Value};

use crate::support::Harness;

async fn post_goal(h: &Harness, title: &str, sort: Option<i64>) -> Value {
    let body = match sort {
        Some(sort) => json!({"title": title, "sort": sort}),
        None => json!({"title": title}),
    };
    let (status, body) = h.req(Method::POST, "/api/project/goals", Some(body)).await;
    assert_eq!(status, 200, "{body}");
    body
}

async fn post_initiative(h: &Harness, goal_id: &str, title: &str) -> Value {
    let (status, body) = h
        .req(
            Method::POST,
            "/api/project/initiatives",
            Some(json!({"goal_id": goal_id, "title": title})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    body
}

async fn post_todo(h: &Harness, initiative_id: Option<&str>, title: &str) -> Value {
    let body = match initiative_id {
        Some(mid) => json!({"initiative_id": mid, "title": title, "draft": "draft"}),
        None => json!({"title": title, "draft": "draft"}),
    };
    let (status, body) = h.req(Method::POST, "/api/project/todos", Some(body)).await;
    assert_eq!(status, 200, "{body}");
    body
}

async fn goals(h: &Harness) -> Vec<Value> {
    let (status, body) = h.req(Method::GET, "/api/project/goals", None).await;
    assert_eq!(status, 200, "{body}");
    body["goals"].as_array().unwrap().clone()
}

async fn initiatives(h: &Harness, goal_id: Option<&str>) -> Vec<Value> {
    let path = match goal_id {
        Some(id) => format!("/api/project/initiatives?goal_id={id}"),
        None => "/api/project/initiatives".to_string(),
    };
    let (status, body) = h.req(Method::GET, &path, None).await;
    assert_eq!(status, 200, "{body}");
    body["initiatives"].as_array().unwrap().clone()
}

async fn todos(h: &Harness) -> Vec<Value> {
    let (status, body) = h.req(Method::GET, "/api/project/todos", None).await;
    assert_eq!(status, 200, "{body}");
    body["todos"].as_array().unwrap().clone()
}

fn row<'a>(rows: &'a [Value], id: &str) -> &'a Value {
    rows.iter()
        .find(|r| r["id"] == json!(id))
        .unwrap_or_else(|| panic!("row {id} missing"))
}

#[path = "project_crud_extra/suite_1.rs"]
mod suite_1;
