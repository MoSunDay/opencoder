use reqwest::Method;
use serde_json::json;

use crate::support::Harness;

#[tokio::test]
async fn initiatives_and_milestones_are_siblings_and_hold_separate_todos() {
    let harness = Harness::new().await;
    let (status, project) = harness
        .req(
            Method::POST,
            "/api/project/goals",
            Some(json!({"title":"P"})),
        )
        .await;
    assert_eq!(status, 200, "{project}");
    let project_id = project["id"].as_str().unwrap();
    let (status, milestone) = harness
        .req(
            Method::POST,
            "/api/project/milestones",
            Some(json!({"goal_id":project_id,"title":"M"})),
        )
        .await;
    assert_eq!(status, 200, "{milestone}");
    let (status, initiative) = harness
        .req(
            Method::POST,
            "/api/project/initiatives",
            Some(json!({"goal_id":project_id,"title":"I"})),
        )
        .await;
    assert_eq!(status, 200, "{initiative}");
    let (status, standalone) = harness
        .req(
            Method::POST,
            "/api/project/initiatives",
            Some(json!({"title":"独立专项"})),
        )
        .await;
    assert_eq!(status, 200, "{standalone}");
    assert!(standalone["goal_id"].is_null());
    assert_ne!(milestone["id"], initiative["id"]);
    for (group, title) in [
        (&milestone, "milestone TODO"),
        (&initiative, "initiative TODO"),
    ] {
        let (status, todo) = harness
            .req(
                Method::POST,
                "/api/project/todos",
                Some(json!({
                    "milestone_id": group["id"], "title": title, "draft": "work"
                })),
            )
            .await;
        assert_eq!(status, 200, "{todo}");
    }
    let (status, overview) = harness
        .req(Method::GET, "/api/project/overview", None)
        .await;
    assert_eq!(status, 200, "{overview}");
    assert_eq!(
        overview["goals"][0]["milestones"][0]["todos"][0]["title"],
        "milestone TODO"
    );
    assert_eq!(
        overview["goals"][0]["initiatives"][0]["todos"][0]["title"],
        "initiative TODO"
    );
    assert_eq!(overview["standalone_initiatives"][0]["title"], "独立专项");
    let (status, _) = harness
        .req(
            Method::DELETE,
            &format!(
                "/api/project/milestones/{}",
                initiative["id"].as_str().unwrap()
            ),
            None,
        )
        .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn todo_execution_links_roundtrip_and_validate_kind() {
    let harness = Harness::new().await;
    let (status, todo) = harness
        .req(
            Method::POST,
            "/api/project/todos",
            Some(json!({"title":"linked todo", "draft":"do it"})),
        )
        .await;
    assert_eq!(status, 200, "{todo}");
    let todo_id = todo["id"].as_str().unwrap();
    let path = format!("/api/project/todos/{todo_id}/executions");

    let (status, body) = harness
        .req(Method::GET, "/api/project/todos/missing/executions", None)
        .await;
    assert_eq!(status, 404, "{body}");

    let (status, body) = harness
        .req(Method::POST, &path, Some(json!({"execution_id":"missing"})))
        .await;
    assert_eq!(status, 404, "{body}");

    let (status, execution) = harness
        .req(
            Method::POST,
            "/api/executions",
            Some(json!({
                "id":"agent-project-link-1", "kind":"agent", "input":{"prompt":"hi"}
            })),
        )
        .await;
    assert_eq!(status, 202, "{execution}");
    let (status, index) = harness
        .req(
            Method::GET,
            "/api/executions/agent-project-link-1/index",
            None,
        )
        .await;
    assert_eq!(status, 200, "{index}");
    assert_eq!(index["kind"], "agent");

    for _ in 0..2 {
        let (status, body) = harness
            .req(
                Method::POST,
                &path,
                Some(json!({"execution_id":"agent-project-link-1"})),
            )
            .await;
        assert_eq!(status, 200, "{body}");
    }
    let (status, body) = harness.req(Method::GET, &path, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["execution_ids"], json!(["agent-project-link-1"]));

    let (status, body) = harness
        .req(
            Method::DELETE,
            &format!("{path}/agent-project-link-1"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = harness.req(Method::GET, &path, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["execution_ids"], json!([]));
    let (status, body) = harness
        .req(
            Method::DELETE,
            &format!("{path}/agent-project-link-1"),
            None,
        )
        .await;
    assert_eq!(status, 404, "{body}");
}
