use crate::support::Harness;
use opencoder_store::project::ProjectAssignment;
use reqwest::Method;
use serde_json::{json, Value};

async fn create(h: &Harness, kind: &str, body: Value) -> Value {
    let (status, value) = h
        .req(Method::POST, &format!("/api/project/{kind}"), Some(body))
        .await;
    assert_eq!(status, 200, "{value}");
    value
}

async fn setup(h: &Harness) -> (String, String, Value) {
    let project = create(h, "goals", json!({"title":"release"})).await;
    let initiative = create(
        h,
        "initiatives",
        json!({"goal_id":project["id"],"title":"work"}),
    )
    .await;
    let todo = create(
        h,
        "todos",
        json!({"initiative_id":initiative["id"],"title":"verify",
        "draft":"requirements\n\n## 当前卡点\nold blocker", "capability_id":"cap",
        "board_status":"in_progress"}),
    )
    .await;
    let dependency = create(
        h,
        "todos",
        json!({"initiative_id":initiative["id"],
        "title":"prerequisite","draft":"done evidence","board_status":"done"}),
    )
    .await;
    let id = todo["id"].as_str().unwrap().to_owned();
    let dep = dependency["id"].as_str().unwrap().to_owned();
    h.state
        .projects
        .link_todo_execution(&ProjectAssignment {
            todo_id: id.clone(),
            execution_id: "operator-accepted".into(),
            capability_id: Some("cap".into()),
            kind: "operator".into(),
            name: "work".into(),
            created_at: 1,
        })
        .await
        .unwrap();
    let patch = json!({"board_status":"done", "draft":"requirements", "acceptance":{
        "title":todo["title"], "draft":todo["draft"], "initiative_id":initiative["id"],
        "project_id":project["id"], "capability_id":"cap", "execution_id":"operator-accepted",
        "required_done":[dep], "all_other_todos":true,
    }});
    (id, dep, patch)
}

#[tokio::test]
async fn acceptance_commits_once_and_retries_after_lost_reply_without_changing_requirements() {
    let h = Harness::new().await;
    let (id, _, patch) = setup(&h).await;
    let path = format!("/api/project/todos/{id}");
    for _ in 0..2 {
        let (status, body) = h.req(Method::PATCH, &path, Some(patch.clone())).await;
        assert_eq!(status, 200, "{body}");
    }
    let (_, current) = h.req(Method::GET, &path, None).await;
    assert_eq!(current["board_status"], "done");
    assert_eq!(current["draft"], "requirements");
    let mut invalid = patch;
    invalid["draft"] = json!("different requirements");
    assert_eq!(h.req(Method::PATCH, &path, Some(invalid)).await.0, 400);
}

#[tokio::test]
async fn ordinary_task_acceptance_does_not_wait_for_unrelated_project_work() {
    let h = Harness::new().await;
    let (id, _, mut patch) = setup(&h).await;
    patch["acceptance"]["all_other_todos"] = json!(false);
    create(
        &h,
        "todos",
        json!({"initiative_id":patch["acceptance"]["initiative_id"],
        "title":"unrelated","draft":"later work","board_status":"todo"}),
    )
    .await;
    let (status, body) = h
        .req(
            Method::PATCH,
            &format!("/api/project/todos/{id}"),
            Some(patch),
        )
        .await;
    assert_eq!(status, 200, "{body}");
}

#[tokio::test]
async fn acceptance_rejects_changes_between_evidence_read_and_commit_without_overwriting_them() {
    for mutation in [
        "draft",
        "title",
        "capability",
        "execution",
        "dependency",
        "new_task",
        "move",
        "unlink",
    ] {
        let h = Harness::new().await;
        let (id, dependency, patch) = setup(&h).await;
        let path = format!("/api/project/todos/{id}");
        // The evidence reader has its snapshot; hold its commit until the edit finishes.
        let (release, wait) = tokio::sync::oneshot::channel();
        let accepting = {
            let h = h.clone();
            let path = path.clone();
            let patch = patch.clone();
            tokio::spawn(async move {
                wait.await.unwrap();
                h.req(Method::PATCH, &path, Some(patch)).await
            })
        };
        match mutation {
            "execution" => h
                .state
                .projects
                .link_todo_execution(&ProjectAssignment {
                    todo_id: id.clone(),
                    execution_id: "operator-newer".into(),
                    capability_id: Some("cap".into()),
                    kind: "operator".into(),
                    name: "new work".into(),
                    created_at: 2,
                })
                .await
                .unwrap(),
            "unlink" => {
                h.state
                    .projects
                    .unlink_todo_execution(&id, "operator-accepted")
                    .await
                    .unwrap();
            }
            "new_task" => {
                create(
                    &h,
                    "todos",
                    json!({"initiative_id":patch["acceptance"]["initiative_id"],
                "title":"new requirement","draft":"must finish","board_status":"todo"}),
                )
                .await;
            }
            _ => {
                let (target, change) = match mutation {
                    "draft" => (
                        path.clone(),
                        json!({"draft":"new requirements and blocker"}),
                    ),
                    "title" => (path.clone(), json!({"title":"changed task"})),
                    "capability" => (path.clone(), json!({"capability_id":"new-cap"})),
                    "move" => (path.clone(), json!({"initiative_id":null})),
                    "dependency" => (
                        format!("/api/project/todos/{dependency}"),
                        json!({"board_status":"todo"}),
                    ),
                    _ => unreachable!(),
                };
                assert_eq!(h.req(Method::PATCH, &target, Some(change)).await.0, 200);
            }
        }
        let (_, before) = h.req(Method::GET, &path, None).await;
        release.send(()).unwrap();
        let (status, error) = accepting.await.unwrap();
        assert_eq!(status, 409, "{mutation}: {error}");
        let (_, after) = h.req(Method::GET, &path, None).await;
        assert_eq!(after, before, "stale acceptance changed {mutation}");
        assert_eq!(after["board_status"], "in_progress");
    }
}
