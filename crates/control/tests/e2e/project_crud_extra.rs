//! Extra CRUD coverage for `/api/project`: goal list order/shape + trimmed
//! create, patch validation and persistence, goal-level cascade, milestone
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

async fn post_milestone(h: &Harness, goal_id: &str, title: &str) -> Value {
    let (status, body) = h
        .req(
            Method::POST,
            "/api/project/milestones",
            Some(json!({"goal_id": goal_id, "title": title})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    body
}

async fn post_todo(h: &Harness, milestone_id: Option<&str>, title: &str) -> Value {
    let body = match milestone_id {
        Some(mid) => json!({"milestone_id": mid, "title": title, "draft": "draft"}),
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

async fn milestones(h: &Harness, goal_id: Option<&str>) -> Vec<Value> {
    let path = match goal_id {
        Some(id) => format!("/api/project/milestones?goal_id={id}"),
        None => "/api/project/milestones".to_string(),
    };
    let (status, body) = h.req(Method::GET, &path, None).await;
    assert_eq!(status, 200, "{body}");
    body["milestones"].as_array().unwrap().clone()
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

#[tokio::test]
async fn goals_list_order_shape_and_trimmed_create() {
    let h = Harness::new().await;
    let first = post_goal(&h, "First", None).await;
    assert_eq!(first["sort"], json!(0));
    assert_eq!(first["status"], json!("active"));
    assert!(first["id"].as_str().unwrap().starts_with("pg-"));

    // Title is trimmed; sort echoes back; the goal starts active.
    let second = post_goal(&h, "  Padded  ", Some(5)).await;
    assert_eq!(second["title"], json!("Padded"));
    assert_eq!(second["sort"], json!(5));
    assert_eq!(second["status"], json!("active"));
    assert!(second["id"].as_str().unwrap().starts_with("pg-"));
    assert_eq!(second["detail_md"], json!(null));

    // List: two goals in `sort` order, each a full record wire form.
    let rows = goals(&h).await;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["id"], first["id"]);
    assert_eq!(rows[0]["title"], json!("First"));
    assert_eq!(rows[1]["id"], second["id"]);
    for key in [
        "id",
        "title",
        "detail_md",
        "status",
        "sort",
        "created_at",
        "updated_at",
    ] {
        assert!(rows[0].get(key).is_some(), "{key} missing: {}", rows[0]);
    }
}

#[tokio::test]
async fn goal_patch_validation_and_persistence() {
    let h = Harness::new().await;
    let goal = post_goal(&h, "G", None).await;
    let id = goal["id"].as_str().unwrap();

    let (status, body) = h
        .req(
            Method::PATCH,
            &format!("/api/project/goals/{id}"),
            Some(json!({"title": "   "})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert!(body["error"].as_str().unwrap().contains("empty"), "{body}");

    let (status, body) = h
        .req(
            Method::PATCH,
            "/api/project/goals/pg-none",
            Some(json!({"title": "x"})),
        )
        .await;
    assert_eq!(status, 404, "{body}");

    // Real fields persist across a re-read.
    let (status, body) = h
        .req(
            Method::PATCH,
            &format!("/api/project/goals/{id}"),
            Some(json!({"title": " G2 ", "detail_md": "d2", "sort": 7})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"], json!(true));
    let rows = goals(&h).await;
    let patched = row(&rows, id);
    assert_eq!(patched["title"], json!("G2"));
    assert_eq!(patched["detail_md"], json!("d2"));
    assert_eq!(patched["sort"], json!(7));

    let (status, body) = h
        .req(Method::DELETE, "/api/project/goals/pg-none", None)
        .await;
    assert_eq!(status, 404, "{body}");
}

#[tokio::test]
async fn goal_delete_cascades_milestones_and_todos() {
    let h = Harness::new().await;
    let goal = post_goal(&h, "G", None).await;
    let goal_id = goal["id"].as_str().unwrap().to_string();
    let milestone = post_milestone(&h, &goal_id, "M").await;
    let milestone_id = milestone["id"].as_str().unwrap().to_string();
    post_todo(&h, Some(&milestone_id), "T").await;

    let (status, body) = h
        .req(
            Method::DELETE,
            &format!("/api/project/goals/{goal_id}"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["deleted"], json!(true));

    assert!(goals(&h).await.is_empty());
    assert!(milestones(&h, None).await.is_empty());
    assert!(todos(&h).await.is_empty(), "goal delete must cascade todos");
}

#[tokio::test]
async fn milestone_create_validation_and_list_filters() {
    let h = Harness::new().await;
    let (status, body) = h
        .req(
            Method::POST,
            "/api/project/milestones",
            Some(json!({"goal_id": "pg-none", "title": "  "})),
        )
        .await;
    assert_eq!(status, 400, "{body}");

    let (status, body) = h
        .req(
            Method::POST,
            "/api/project/milestones",
            Some(json!({"goal_id": "pg-none", "title": "M"})),
        )
        .await;
    assert_eq!(status, 404, "{body}");
    assert!(
        body["error"].as_str().unwrap().contains("pg-none"),
        "{body}"
    );

    let goal = post_goal(&h, "G", None).await;
    let goal_id = goal["id"].as_str().unwrap();
    let first = post_milestone(&h, goal_id, "M1").await;
    post_milestone(&h, goal_id, "M2").await;
    assert!(first["id"].as_str().unwrap().starts_with("pm-"));
    assert_eq!(first["status"], json!("planned"));

    assert_eq!(milestones(&h, None).await.len(), 2);
    assert_eq!(milestones(&h, Some("pg-none")).await.len(), 0);
    assert_eq!(milestones(&h, Some(goal_id)).await.len(), 2);
}

#[tokio::test]
async fn milestone_patch_status_sort_and_reparent() {
    let h = Harness::new().await;
    let g1 = post_goal(&h, "G1", None).await;
    let g2 = post_goal(&h, "G2", None).await;
    let g1_id = g1["id"].as_str().unwrap().to_string();
    let g2_id = g2["id"].as_str().unwrap().to_string();
    let m = post_milestone(&h, &g1_id, "M").await;
    let m_id = m["id"].as_str().unwrap().to_string();

    let (status, body) = h
        .req(
            Method::PATCH,
            &format!("/api/project/milestones/{m_id}"),
            Some(json!({"status": "in_progress", "sort": 2})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let rows = milestones(&h, Some(&g1_id)).await;
    let patched = row(&rows, &m_id);
    assert_eq!(patched["status"], json!("in_progress"));
    assert_eq!(patched["sort"], json!(2));

    // Re-parenting to another real goal moves the row.
    let (status, body) = h
        .req(
            Method::PATCH,
            &format!("/api/project/milestones/{m_id}"),
            Some(json!({"goal_id": g2_id})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(milestones(&h, Some(&g1_id)).await.len(), 0);
    let moved = milestones(&h, Some(&g2_id)).await;
    assert_eq!(moved.len(), 1);
    assert_eq!(row(&moved, &m_id)["goal_id"], json!(g2_id));

    let (status, body) = h
        .req(
            Method::PATCH,
            &format!("/api/project/milestones/{m_id}"),
            Some(json!({"goal_id": "pg-none"})),
        )
        .await;
    assert_eq!(status, 404, "{body}");
}

#[tokio::test]
async fn todo_create_validation_defaults_and_backlog_listing() {
    let h = Harness::new().await;
    let (status, body) = h
        .req(
            Method::POST,
            "/api/project/todos",
            Some(json!({"title": " ", "draft": "d"})),
        )
        .await;
    assert_eq!(status, 400, "{body}");

    let (status, body) = h
        .req(
            Method::POST,
            "/api/project/todos",
            Some(json!({"milestone_id": "pm-none", "title": "T", "draft": "d"})),
        )
        .await;
    assert_eq!(status, 404, "{body}");
    assert!(
        body["error"].as_str().unwrap().contains("pm-none"),
        "{body}"
    );

    // Absent agent defaults to `act`; absent milestone ⇒ backlog.
    let backlog = post_todo(&h, None, "B1").await;
    assert!(backlog["id"].as_str().unwrap().starts_with("pt-"));
    assert_eq!(backlog["agent"], json!("act"));
    assert_eq!(backlog["status"], json!("draft"));
    assert_eq!(backlog["milestone_id"], json!(null));

    let goal = post_goal(&h, "G", None).await;
    let milestone = post_milestone(&h, goal["id"].as_str().unwrap(), "M").await;
    post_todo(&h, Some(milestone["id"].as_str().unwrap()), "T1").await;

    let rows = todos(&h).await;
    assert_eq!(rows.len(), 2, "unfiltered list must include the backlog");
    assert!(rows.iter().any(|r| r["id"] == backlog["id"]));
}

#[tokio::test]
async fn todo_patch_real_fields_and_reparent() {
    let h = Harness::new().await;
    let todo = post_todo(&h, None, "T").await;
    let id = todo["id"].as_str().unwrap().to_string();

    let (status, body) = h
        .req(
            Method::PATCH,
            &format!("/api/project/todos/{id}"),
            Some(json!({"title": "renamed", "draft": "d2", "agent": "build"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"], json!(true));
    let rows = todos(&h).await;
    let patched = row(&rows, &id);
    assert_eq!(patched["title"], json!("renamed"));
    assert_eq!(patched["draft"], json!("d2"));
    assert_eq!(patched["agent"], json!("build"));

    let (status, body) = h
        .req(
            Method::PATCH,
            &format!("/api/project/todos/{id}"),
            Some(json!({"title": "  "})),
        )
        .await;
    assert_eq!(status, 400, "{body}");

    let (status, body) = h
        .req(
            Method::PATCH,
            &format!("/api/project/todos/{id}"),
            Some(json!({"milestone_id": "pm-none"})),
        )
        .await;
    assert_eq!(status, 404, "{body}");

    // Re-parent into a real milestone persists (backlog → milestone).
    let goal = post_goal(&h, "G", None).await;
    let milestone = post_milestone(&h, goal["id"].as_str().unwrap(), "M").await;
    let m_id = milestone["id"].as_str().unwrap().to_string();
    let (status, body) = h
        .req(
            Method::PATCH,
            &format!("/api/project/todos/{id}"),
            Some(json!({"milestone_id": m_id})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let rows = todos(&h).await;
    assert_eq!(row(&rows, &id)["milestone_id"], json!(m_id));

    let (status, body) = h
        .req(
            Method::PATCH,
            "/api/project/todos/pt-none",
            Some(json!({"title": "x"})),
        )
        .await;
    assert_eq!(status, 404, "{body}");
}
