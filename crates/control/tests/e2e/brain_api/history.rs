use crate::support::Harness;
use opencoder_core::fleet::{
    Assignment, CreateExecution, ExecutionIndex, ExecutionKind, ExecutionStatus,
};
use reqwest::Method;
use serde_json::{json, Value};

async fn seed(h: &Harness, id: &str, created_at: i64, title: Option<&str>) {
    let index = ExecutionIndex {
        id: id.into(),
        created_at,
        kind: ExecutionKind::Brain,
        node_id: "offline-node".into(),
        status: ExecutionStatus::Done,
    };
    h.state.fleet.put_index(&index).await.unwrap();
    if let Some(title) = title {
        let assignment = Assignment {
            index,
            private_context: None,
            runtime: None,
            codex: None,
            definition: None,
            request: CreateExecution {
                id: id.into(),
                kind: ExecutionKind::Brain,
                target: None,
                node_id: None,
                input: json!({"schema_version":7,"layered_request":{"plan":{"title":title}}}),
            },
        };
        h.state
            .fleet
            .claim_request("execution", id, id)
            .await
            .unwrap();
        h.state
            .fleet
            .prepare_assignment(&assignment, id)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn history_uses_frozen_names_and_cursor_without_a_live_node() {
    let h = Harness::new().await;
    seed(&h, "brain-old", 1, Some("执行时名称")).await;
    seed(&h, "brain-missing", 2, None).await;
    seed(&h, "brain-new", 3, Some("新计划名称")).await;
    let (status, page) = h.req(Method::GET, "/api/brain/runs?limit=2", None).await;
    assert_eq!(status, 200, "{page}");
    let rows = page["runs"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["name"], "新计划名称");
    assert_eq!(rows[1]["id"], "brain-missing");
    assert!(rows[1].get("name").is_none());
    let cursor = &page["next_cursor"];
    let route = format!(
        "/api/brain/runs?limit=2&cursor_created_at={}&cursor_id={}",
        cursor["created_at"],
        cursor["id"].as_str().unwrap()
    );
    let (status, older) = h.req(Method::GET, &route, None).await;
    assert_eq!(status, 200, "{older}");
    assert_eq!(older["runs"][0]["name"], "执行时名称");
    assert_eq!(older["next_cursor"], Value::Null);
    for route in [
        "/api/executions/brain-old/index",
        "/api/executions?kind=brain",
    ] {
        let (status, body) = h.req(Method::GET, route, None).await;
        assert_eq!(status, 200, "{body}");
        if route.ends_with("/index") {
            assert_eq!(body["name"], "执行时名称");
        } else {
            assert!(body["executions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["name"] == "执行时名称"));
        }
    }
    for query in [
        "limit=0",
        "limit=999999",
        "cursor_id=brain-old",
        "cursor_created_at=1",
    ] {
        let (status, body) = h
            .req(Method::GET, &format!("/api/brain/runs?{query}"), None)
            .await;
        assert_eq!(status, 400, "{body}");
    }
}
