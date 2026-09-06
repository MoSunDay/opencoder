//! Execution core: create (202 receipt, validation, conflicts, idempotent
//! retry), list filters/paging, inspect and command dispatch, event payload
//! and detail-field paging endpoints.

use opencoder_core::fleet::{ExecutionKind, ExecutionStatus};
use reqwest::Method;
use serde_json::json;

use crate::support::Harness;

async fn create_ok(h: &Harness, id: &str) -> (reqwest::StatusCode, serde_json::Value) {
    h.req(
        Method::POST,
        "/api/executions",
        Some(json!({"id": id, "kind": "agent", "input": {"prompt": "hi"}})),
    )
    .await
}

#[tokio::test]
async fn create_returns_202_with_durable_five_field_receipt() {
    let h = Harness::new().await;
    let (status, receipt) = create_ok(&h, "agent-create-1").await;
    assert_eq!(status, 202, "{receipt}");
    assert_eq!(receipt["id"], json!("agent-create-1"));
    assert_eq!(receipt["kind"], json!("agent"));
    assert_eq!(receipt["node_id"], json!("node-e2e"));
    assert_eq!(receipt["status"], json!("pending"));
    assert!(receipt["created_at"].as_i64().unwrap_or(0) > 0);

    // The control-plane index now lists it with the five fields.
    let (list_status, list_body) = h.req(Method::GET, "/api/executions", None).await;
    assert_eq!(list_status, 200, "{list_body}");
    let execs = list_body["executions"].as_array().unwrap();
    let mine = execs
        .iter()
        .find(|e| e["id"] == json!("agent-create-1"))
        .unwrap();
    assert_eq!(mine["kind"], json!("agent"));
    assert_eq!(mine["node_id"], json!("node-e2e"));

    // Idempotent retry with the same id + identical input → same receipt.
    let (status, again) = create_ok(&h, "agent-create-1").await;
    assert_eq!(status, 202, "{again}");
    assert_eq!(again["id"], receipt["id"]);
    assert_eq!(again["created_at"], receipt["created_at"]);
}

#[tokio::test]
async fn create_rejects_invalid_ids_kinds_and_projects() {
    let h = Harness::new().await;
    // Wrong prefix for the kind.
    let (status, body) = h
        .req(
            Method::POST,
            "/api/executions",
            Some(json!({"id": "wrong-1", "kind": "agent"})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("id must start with"),
        "{body}"
    );
    // System executions are retired.
    let (status, body) = h
        .req(
            Method::POST,
            "/api/executions",
            Some(json!({"id": "team-1", "kind": "system"})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert!(
        body["error"].as_str().unwrap().contains("retired"),
        "{body}"
    );
    // Project ids must be project-<target>.
    let (status, body) = h
        .req(
            Method::POST,
            "/api/executions",
            Some(json!({"id": "project-other", "kind": "project", "target": "todo-9"})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("project-<todo id>"),
        "{body}"
    );
    // Conflicting kind / node for an existing id. The id prefix validation
    // runs before the conflict check, so the cross-kind index state cannot
    // be constructed via the API and is pre-seeded directly instead (the
    // conflicting request itself must still pass prefix validation).
    h.put_index(
        "agent-conflict-1",
        ExecutionKind::Dag,
        ExecutionStatus::Idle,
    )
    .await;
    let (status, body) = h
        .req(
            Method::POST,
            "/api/executions",
            Some(json!({"id": "agent-conflict-1", "kind": "agent", "target": "x"})),
        )
        .await;
    assert_eq!(status, 409, "{body}");
    let (status, body) = h
        .req(
            Method::POST,
            "/api/executions",
            Some(json!({"id": "agent-conflict-1", "kind": "agent", "node_id": "node-b"})),
        )
        .await;
    assert_eq!(status, 409, "{body}");
}

#[tokio::test]
async fn list_filters_kinds_nodes_and_validates_paging() {
    let h = Harness::new().await;
    h.put_index("agent-l1", ExecutionKind::Agent, ExecutionStatus::Idle)
        .await;
    h.put_index("agent-l2", ExecutionKind::Agent, ExecutionStatus::Running)
        .await;
    h.put_index("dag-l3", ExecutionKind::Dag, ExecutionStatus::Running)
        .await;
    let (status, body) = h.req(Method::GET, "/api/executions?kind=agent", None).await;
    assert_eq!(status, 200, "{body}");
    let ids: Vec<_> = body["executions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap().to_string())
        .collect();
    assert!(
        ids.contains(&"agent-l1".into()) && ids.contains(&"agent-l2".into()),
        "{ids:?}"
    );
    assert!(!ids.iter().any(|i| i == "dag-l3"), "{ids:?}");

    let (status, body) = h
        .req(Method::GET, "/api/executions?kind=agent&limit=1", None)
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["executions"].as_array().unwrap().len(), 1);
    assert!(body["next_cursor"]["id"].as_str().is_some(), "{body}");

    let (status, body) = h.req(Method::GET, "/api/executions?limit=0", None).await;
    assert_eq!(status, 400, "{body}");
    let (status, body) = h.req(Method::GET, "/api/executions?kind=bogus", None).await;
    assert_eq!(status, 400, "{body}");
    let (status, body) = h
        .req(Method::GET, "/api/executions?node_id=ghost", None)
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["executions"], json!([]));
}

#[tokio::test]
async fn inspect_and_commands_route_by_id_to_the_owning_node() {
    let h = Harness::new().await;
    h.put_index("agent-insp-1", ExecutionKind::Agent, ExecutionStatus::Idle)
        .await;
    h.node.set_inspect(
        "agent-insp-1",
        json!({"execution": {"id": "agent-insp-1", "status": "idle"}, "session": {"title": "t"}}),
    );
    h.node.set_command(
        "agent-insp-1",
        "cancel",
        200,
        json!({"id": "agent-insp-1", "status": "cancelling"}),
    );

    let (status, body) = h
        .req(Method::GET, "/api/executions/agent-insp-1", None)
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["session"]["title"], json!("t"));

    let (status, body) = h
        .req(
            Method::POST,
            "/api/executions/agent-insp-1/commands",
            Some(json!({"action": "cancel"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["status"], json!("cancelling"));

    // Unknown id → control-plane 404 before any node call.
    let (status, body) = h.req(Method::GET, "/api/executions/agent-none", None).await;
    assert_eq!(status, 404, "{body}");
    // Node-side rejection is passed through verbatim.
    let (status, body) = h
        .req(
            Method::POST,
            "/api/executions/agent-insp-1/commands",
            Some(json!({"action": "wat"})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"], json!("unknown execution command"));

    // An id that only exists in the control-plane index (no node journal or
    // inspect entry) surfaces the node's authoritative miss.
    h.put_index("agent-lost-1", ExecutionKind::Agent, ExecutionStatus::Idle)
        .await;
    let (status, body) = h
        .req(Method::GET, "/api/executions/agent-lost-1", None)
        .await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["error"], json!("execution not found"));

    // Commands for an unknown id are a control-plane 404 before any node call.
    let (status, body) = h
        .req(
            Method::POST,
            "/api/executions/agent-none/commands",
            Some(json!({"action": "cancel"})),
        )
        .await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["error"], json!("execution id not found"));
    assert!(
        h.node
            .seen_commands()
            .iter()
            .all(|(id, _, _)| id != "agent-none"),
        "no command may reach the node for an unknown id"
    );
}

/// Seeds a project todo through the public API and returns its id.
async fn seed_project_todo(h: &Harness) -> String {
    let (status, body) = h
        .req(
            Method::POST,
            "/api/project/todos",
            Some(json!({"title": "T1", "draft": "do it"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    body["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn project_plan_execute_commands_carry_the_plan_snapshot() {
    let h = Harness::new().await;
    let todo_id = seed_project_todo(&h).await;
    let exec_id = format!("project-{todo_id}");
    // The executions command surface requires an existing execution id, so
    // the project execution is created through the submit route first.
    let (status, body) = h
        .req(
            Method::POST,
            "/api/executions",
            Some(json!({"id": exec_id, "kind": "project", "target": todo_id})),
        )
        .await;
    assert_eq!(status, 202, "{body}");
    h.node.set_command(
        &exec_id,
        "plan",
        200,
        json!({"id": exec_id, "status": "planning"}),
    );
    h.node.set_command(
        &exec_id,
        "execute",
        200,
        json!({"id": exec_id, "status": "running"}),
    );

    let (status, body) = h
        .req(
            Method::POST,
            &format!("/api/executions/{exec_id}/commands"),
            Some(json!({"action": "plan"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["status"], json!("planning"));
    let (status, body) = h
        .req(
            Method::POST,
            &format!("/api/executions/{exec_id}/commands"),
            Some(json!({"action": "execute"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["status"], json!("running"));

    // The plan command reaches the node with the authoritative plan snapshot
    // injected; execute rides the same affinity without re-resolving it.
    let seen = h.node.seen_commands();
    let plan = seen
        .iter()
        .find(|(id, action, _)| id == &exec_id && action == "plan")
        .expect("plan command forwarded to the node");
    assert!(
        plan.2["snapshot"].is_object(),
        "snapshot injected: {plan:?}"
    );
    assert!(plan.2["snapshot"]["todo"].is_object(), "{plan:?}");
    assert!(plan.2["snapshot"]["goals"].is_array(), "{plan:?}");
    assert!(plan.2["snapshot"]["milestones"].is_array(), "{plan:?}");
    let execute = seen
        .iter()
        .find(|(id, action, _)| id == &exec_id && action == "execute")
        .expect("execute command forwarded to the node");
    assert!(
        execute.2.get("snapshot").is_none(),
        "execute carries no snapshot: {execute:?}"
    );

    // plan for an unknown project todo id resolves to a clean 404.
    let (status, body) = h
        .req(
            Method::POST,
            "/api/executions/project-ghost/commands",
            Some(json!({"action": "plan"})),
        )
        .await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["error"], json!("todo not found"));
}

#[tokio::test]
async fn plan_execute_and_system_commands_are_gated_by_id_rules() {
    let h = Harness::new().await;
    h.put_index("agent-cmd-1", ExecutionKind::Agent, ExecutionStatus::Idle)
        .await;
    let (status, body) = h
        .req(
            Method::POST,
            "/api/executions/agent-cmd-1/commands",
            Some(json!({"action": "plan"})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(
        body["error"],
        json!("plan/execute requires project execution")
    );

    h.put_index("team-sys-1", ExecutionKind::System, ExecutionStatus::Idle)
        .await;
    h.node.set_command(
        "team-sys-1",
        "interrupt",
        200,
        json!({"id": "team-sys-1", "status": "cancelled"}),
    );
    let (status, body) = h
        .req(
            Method::POST,
            "/api/executions/team-sys-1/commands",
            Some(json!({"action": "prompt"})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    let (status, body) = h
        .req(
            Method::POST,
            "/api/executions/team-sys-1/commands",
            Some(json!({"action": "interrupt"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["status"], json!("cancelled"));
}

#[tokio::test]
async fn event_payload_and_detail_field_paging_endpoints() {
    let h = Harness::new().await;
    h.put_index("agent-page-1", ExecutionKind::Agent, ExecutionStatus::Idle)
        .await;
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode("payload-bytes");
    h.node.set_payload(
        "agent-page-1",
        7,
        json!({"seq": 7, "offset": 0, "next_offset": 13, "total_bytes": 13, "eof": true, "encoding": "base64", "bytes_b64": b64}),
    );
    h.node.set_field(
        "agent-page-1",
        "request.input",
        json!({"field": "request.input", "offset": 0, "next_offset": 4, "total_bytes": 4, "eof": true, "encoding": "json-base64", "bytes_b64": b64}),
    );
    let (status, body) = h
        .req(
            Method::GET,
            "/api/executions/agent-page-1/events/7/payload",
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["seq"], json!(7));
    assert_eq!(body["total_bytes"], json!(13));
    let (status, body) = h
        .req(
            Method::GET,
            "/api/executions/agent-page-1/events/0/payload",
            None,
        )
        .await;
    assert_eq!(status, 400, "{body}");
    let (status, body) = h
        .req(
            Method::GET,
            "/api/executions/agent-page-1/events/99/payload",
            None,
        )
        .await;
    assert_eq!(status, 404, "{body}");

    let (status, body) = h
        .req(
            Method::GET,
            "/api/executions/agent-page-1/detail-field?field=request.input",
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["field"], json!("request.input"));
    assert_eq!(body["encoding"], json!("json-base64"));
    let (status, body) = h
        .req(
            Method::GET,
            "/api/executions/agent-page-1/detail-field?field=bad%20field",
            None,
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"], json!("invalid detail field"));
    let (status, body) = h
        .req(
            Method::GET,
            "/api/executions/agent-page-1/detail-field?field=result",
            None,
        )
        .await;
    assert_eq!(status, 404, "{body}");
}
