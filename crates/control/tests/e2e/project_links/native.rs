use crate::support::{Harness, SHARE_GATE};
use reqwest::Method;
use serde_json::{json, Value};

async fn todo(h: &Harness) -> String {
    let (status, row) = h
        .req(
            Method::POST,
            "/api/project/todos",
            Some(json!({
                "title": "shared project task",
                "draft": "work",
                "capability_id": "builtin-agent-act"
            })),
        )
        .await;
    assert_eq!(status, 200, "{row}");
    format!("/api/project/todos/{}", row["id"].as_str().unwrap())
}

fn workflow_spec(name: &str) -> Value {
    json!({
        "schema_version": 1,
        "id": format!("project-{name}"),
        "name": name,
        "objective": "run the project workflow",
        "todos": [{
            "id": "step",
            "title": "project step",
            "requirement_background": "project input",
            "instructions": "complete the project step",
            "agent": "act",
            "acceptance": {"criteria": "completed"}
        }],
        "metadata": {},
    })
}

#[tokio::test]
async fn team_and_todo_workflow_capabilities_are_dispatched_and_linked() {
    let _guard = SHARE_GATE.lock().await;
    let share = std::env::temp_dir().join(format!(
        "oc-ctl-e2e-project-dispatch-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&share).unwrap();
    opencoder_core::set_share_dir_override(Some(share.clone()));
    let agents = share.join("agents-root");
    std::fs::create_dir_all(&agents).unwrap();
    opencoder_core::agent::set_agents_dir_override(Some(agents));

    let h = Harness::new().await;
    let (status, team) = h
        .req(
            Method::POST,
            "/api/teams",
            Some(json!({
                "name": "project-team",
                "captain": "act",
                "members": [{"agent": "act"}]
            })),
        )
        .await;
    assert_eq!(status, 200, "{team}");

    let (status, workflow) = h
        .req(
            Method::POST,
            "/api/todo/templates",
            Some(json!({"name": "project-workflow", "spec": workflow_spec("project-workflow")})),
        )
        .await;
    assert_eq!(status, 200, "{workflow}");

    let (_, catalog) = h.req(Method::GET, "/api/brain/library", None).await;
    let capabilities = catalog["capabilities"].as_array().unwrap();
    let team_capability = capabilities
        .iter()
        .find(|capability| capability["id"] == "team-project-team")
        .cloned()
        .expect("team capability must be listed");
    let todos_capability = capabilities
        .iter()
        .find(|capability| capability["id"] == "todos-project-workflow-v1")
        .cloned()
        .expect("TODO workflow capability must be listed");
    assert!(
        team_capability["definition"].is_object(),
        "{team_capability}"
    );
    assert!(
        todos_capability["definition"].is_object(),
        "{todos_capability}"
    );

    let (_, todo) = h
        .req(
            Method::POST,
            "/api/project/todos",
            Some(json!({
                "title": "native capability routing",
                "draft": "run both native capability kinds",
                "capability_id": team_capability["id"]
            })),
        )
        .await;
    let path = format!("/api/project/todos/{}", todo["id"].as_str().unwrap());

    let (status, team_reply) = h
        .dispatch(
            Method::POST,
            &format!("{path}/dispatch"),
            Some(json!({
                "execution_id": "team-project-routing",
                "capability_id": team_capability["id"],
                "input": {"prompt": "team project work"}
            })),
        )
        .await;
    assert_eq!(status, 202, "{team_reply}");
    assert_eq!(team_reply["linked"], true);

    let (status, todos_reply) = h
        .dispatch(
            Method::POST,
            &format!("{path}/dispatch"),
            Some(json!({
                "execution_id": "todos-project-routing",
                "capability_id": todos_capability["id"],
                "input": {"prompt": "workflow project work"}
            })),
        )
        .await;
    assert_eq!(status, 202, "{todos_reply}");
    assert_eq!(todos_reply["linked"], true);

    let team_request = h.node.journal_request("team-project-routing").unwrap();
    assert_eq!(team_request["kind"], "team");
    assert_eq!(team_request["target"], "project-team");
    assert_eq!(team_request["input"]["definition"]["name"], "project-team");

    let todos_request = h.node.journal_request("todos-project-routing").unwrap();
    assert_eq!(todos_request["kind"], "todos");
    assert_eq!(todos_request["target"], "project-workflow/v1");
    assert_eq!(todos_request["input"]["spec"]["name"], "project-workflow");

    let (_, links) = h
        .req(Method::GET, &format!("{path}/executions"), None)
        .await;
    let assignments = links["assignments"].as_array().unwrap();
    assert_eq!(assignments.len(), 2, "{links}");
    assert!(assignments.iter().any(|assignment| {
        assignment["execution_id"] == "team-project-routing"
            && assignment["capability_id"] == "team-project-team"
            && assignment["kind"] == "team"
    }));
    assert!(assignments.iter().any(|assignment| {
        assignment["execution_id"] == "todos-project-routing"
            && assignment["capability_id"] == "todos-project-workflow-v1"
            && assignment["kind"] == "todos"
    }));
}

#[tokio::test]
async fn dispatch_rejects_stale_capabilities_and_invalid_native_requests() {
    let h = Harness::new().await;
    let path = todo(&h).await;
    let dispatch_path = format!("{path}/dispatch");

    let (status, body) = h
        .req(
            Method::POST,
            &dispatch_path,
            Some(json!({
                "execution_id": "agent-project-stale",
                "capability_id": "capability-no-longer-exists",
                "input": {"prompt": "work"}
            })),
        )
        .await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["error"], "capability not found");

    h.state
        .fleet
        .put_definition(
            "brain_capability",
            "required-prompt",
            &json!({
                "id": "required-prompt",
                "kind": "agent",
                "target": "act",
                "required_inputs": ["prompt"],
                "definition": {"name": "act"}
            }),
        )
        .await
        .unwrap();
    let (status, body) = h
        .req(
            Method::POST,
            &dispatch_path,
            Some(json!({
                "execution_id": "agent-project-missing-input",
                "capability_id": "required-prompt",
                "input": {}
            })),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("required capability input"),
        "{body}"
    );

    let (status, body) = h
        .req(
            Method::POST,
            &dispatch_path,
            Some(json!({
                "execution_id": "operator-project-wrong-prefix",
                "capability_id": "builtin-agent-act",
                "input": {"prompt": "work"}
            })),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert!(body["error"].as_str().unwrap().contains("prefix"));
    assert!(h.node.journal_ids().is_empty());
}
