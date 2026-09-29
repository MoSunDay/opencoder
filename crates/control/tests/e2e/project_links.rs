use opencoder_core::fleet::{ExecutionKind, ExecutionStatus};
use reqwest::Method;
use serde_json::json;

use crate::support::Harness;

#[tokio::test]
async fn assignment_link_is_idempotent_and_records_kind_name_and_result() {
    let harness = Harness::new().await;
    let (status, todo) = harness
        .req(
            Method::POST,
            "/api/project/todos",
            Some(json!({
                "title":"route", "draft":"work"
            })),
        )
        .await;
    assert_eq!(status, 200, "{todo}");
    let todo_id = todo["id"].as_str().unwrap();
    harness
        .put_index("agent-linked", ExecutionKind::Agent, ExecutionStatus::Done)
        .await;
    let path = format!("/api/project/todos/{todo_id}/executions");
    for _ in 0..2 {
        let (status, body) = harness
            .req(
                Method::POST,
                &path,
                Some(json!({"execution_id":"agent-linked"})),
            )
            .await;
        assert_eq!(status, 200, "{body}");
    }
    let (status, body) = harness.req(Method::GET, &path, None).await;
    assert_eq!(status, 200, "{body}");
    let assignments = body["assignments"].as_array().unwrap();
    assert_eq!(assignments.len(), 1);
    assert_eq!(assignments[0]["kind"], "agent");
    assert_eq!(assignments[0]["execution_id"], "agent-linked");
    assert_eq!(body["execution_ids"], json!(["agent-linked"]));
    harness
        .state
        .projects
        .finish_todo_assignment(todo_id, "agent-linked", "complete", Some("done"))
        .await
        .unwrap();
    let (_, body) = harness.req(Method::GET, &path, None).await;
    assert_eq!(body["assignments"][0]["result_md"], "done");
    let (_, overview) = harness
        .req(Method::GET, "/api/project/overview", None)
        .await;
    let board = overview["backlog"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == todo_id)
        .unwrap();
    assert_eq!(board["latest_assignment"]["execution_id"], "agent-linked");
    assert_eq!(board["latest_assignment"]["has_result"], true);
}

#[tokio::test]
async fn unknown_execution_cannot_be_assigned() {
    let harness = Harness::new().await;
    let (_, todo) = harness
        .req(
            Method::POST,
            "/api/project/todos",
            Some(json!({"title":"route", "draft":"work"})),
        )
        .await;
    let path = format!(
        "/api/project/todos/{}/executions",
        todo["id"].as_str().unwrap()
    );
    let (status, _) = harness
        .req(Method::POST, &path, Some(json!({"execution_id":"missing"})))
        .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn all_six_native_capabilities_can_be_assigned() {
    let harness = Harness::new().await;
    let (_, todo) = harness
        .req(
            Method::POST,
            "/api/project/todos",
            Some(json!({"title":"route", "draft":"work"})),
        )
        .await;
    let path = format!(
        "/api/project/todos/{}/executions",
        todo["id"].as_str().unwrap()
    );
    for (id, kind) in [
        ("agent-linked", ExecutionKind::Agent),
        ("team-linked", ExecutionKind::Team),
        ("dag-linked", ExecutionKind::Dag),
        ("todos-linked", ExecutionKind::Todos),
        ("brain-linked", ExecutionKind::Brain),
        ("operator-linked", ExecutionKind::Operator),
    ] {
        harness.put_index(id, kind, ExecutionStatus::Pending).await;
        let (status, body) = harness
            .req(Method::POST, &path, Some(json!({"execution_id":id})))
            .await;
        assert_eq!(status, 200, "{body}");
    }
    let (_, body) = harness.req(Method::GET, &path, None).await;
    assert_eq!(body["assignments"].as_array().unwrap().len(), 6);
}
