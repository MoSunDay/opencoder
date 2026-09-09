//! Legacy node-facing compat API: models/skills via maintenance, task
//! create/continue with node pinning, dialogs ledger, cancel and task SSE.

use opencoder_core::fleet::{ExecutionKind, ExecutionStatus};
use reqwest::Method;
use serde_json::json;

use crate::support::Harness;

#[tokio::test]
async fn models_and_skills_delegate_to_node_maintenance() {
    let h = Harness::new().await;
    h.node
        .set_maintenance("models", 200, json!({"models": [{"id": "glm-5.2"}]}));
    h.node
        .set_maintenance("skills", 200, json!({"skills": [{"name": "pdf"}]}));
    let (status, body) = h.req(Method::GET, "/api/models", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["models"][0]["id"], json!("glm-5.2"));
    let (status, body) = h.req(Method::GET, "/api/skills", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["skills"][0]["name"], json!("pdf"));
    let (status, body) = h.req(Method::GET, "/api/models?node_id=ghost", None).await;
    assert_eq!(status, 503, "{body}");
}

#[tokio::test]
async fn task_create_pins_node_and_continue_reuses_session() {
    let h = Harness::new().await;
    // New task: submit routed at the path node → 200 receipt triple.
    let (status, body) = h
        .req(
            Method::POST,
            "/api/nodes/node-e2e/tasks",
            Some(json!({"prompt": "do it", "agent": "act"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let task_id = body["task_id"].as_str().unwrap().to_string();
    assert!(task_id.starts_with("agent-"), "{body}");
    assert_eq!(body["node_id"], json!("node-e2e"));
    assert_eq!(body["session_id"], json!(task_id));

    // Continue: same session id → `prompt` command on the owning node.
    h.node
        .set_command(&task_id, "prompt", 202, json!({"queued": true}));
    let (status, body) = h
        .req(
            Method::POST,
            "/api/nodes/node-e2e/tasks",
            Some(json!({"session_id": task_id, "prompt": "again"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["task_id"], json!(task_id));
    assert_eq!(body["session_id"], json!(task_id));
    assert_eq!(body["node_id"], json!("node-e2e"));
    let seen = h.node.seen_commands();
    assert!(
        seen.iter().any(|(id, action, input)| id == &task_id
            && action == "prompt"
            && input["prompt"] == json!("again")),
        "{seen:?}"
    );

    // Cross-node continue is refused.
    let (status, body) = h
        .req(
            Method::POST,
            "/api/nodes/node-other/tasks",
            Some(json!({"session_id": task_id, "prompt": "x"})),
        )
        .await;
    assert_eq!(status, 409, "{body}");
}

#[tokio::test]
async fn dialogs_ledger_and_task_cancel() {
    let h = Harness::new().await;
    h.put_index("agent-dlg-1", ExecutionKind::Agent, ExecutionStatus::Idle)
        .await;
    h.node.set_command(
        "agent-dlg-1",
        "summary",
        200,
        json!({"id": "agent-dlg-1", "title": "ledger", "status": "idle", "last_created_at": 42}),
    );
    let (status, body) = h
        .req(Method::GET, "/api/nodes/node-e2e/dialogs", None)
        .await;
    assert_eq!(status, 200, "{body}");
    let dialogs = body["dialogs"].as_array().unwrap();
    assert_eq!(dialogs[0]["session_id"], json!("agent-dlg-1"));
    assert_eq!(dialogs[0]["title"], json!("ledger"));

    h.node.set_command(
        "agent-dlg-1",
        "cancel",
        200,
        json!({"id": "agent-dlg-1", "status": "cancelled"}),
    );
    let (status, body) = h
        .req(
            Method::POST,
            "/api/nodes/node-e2e/tasks/agent-dlg-1/cancel",
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["status"], json!("cancelled"));
    // Cancelling from the wrong node is a 409, not a silent redirect.
    let (status, body) = h
        .req(
            Method::POST,
            "/api/nodes/node-other/tasks/agent-dlg-1/cancel",
            None,
        )
        .await;
    assert_eq!(status, 409, "{body}");
}

#[tokio::test]
async fn task_events_stream_uses_the_shared_sse_handler() {
    let h = Harness::new().await;
    h.put_index("agent-tevt-1", ExecutionKind::Agent, ExecutionStatus::Done)
        .await;
    h.node.set_events(
        "agent-tevt-1",
        vec![json!({"seq": 1, "kind": "status", "data": {"phase": "done"}, "ts": 9})],
        true,
    );
    let (status, text) = h.sse_text("/api/nodes/tasks/agent-tevt-1/events").await;
    assert_eq!(status, 200);
    assert!(
        text.contains("id: 1") && text.contains("event: status"),
        "{text}"
    );
}

#[tokio::test]
async fn task_create_validates_caller_ids_and_node_pins() {
    let h = Harness::new().await;
    // Caller-supplied id is honored in the receipt triple.
    let (status, body) = h
        .req(
            Method::POST,
            "/api/nodes/node-e2e/tasks",
            Some(json!({"id": "agent-task-named", "prompt": "hi"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["task_id"], json!("agent-task-named"));
    assert_eq!(body["session_id"], json!("agent-task-named"));
    assert_eq!(body["node_id"], json!("node-e2e"));

    // Malformed caller ids never reach a node.
    let (status, body) = h
        .req(
            Method::POST,
            "/api/nodes/node-e2e/tasks",
            Some(json!({"id": "bad id", "prompt": "x"})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    // A path node that never connected has no eligible fleet member.
    let (status, body) = h
        .req(
            Method::POST,
            "/api/nodes/node-other/tasks",
            Some(json!({"prompt": "hi"})),
        )
        .await;
    assert_eq!(status, 503, "{body}");
    assert_eq!(
        body["error"],
        json!("no ready online node can accept this execution")
    );

    // Continuing an unknown conversation on this node is a 409, not a create.
    let (status, body) = h
        .req(
            Method::POST,
            "/api/nodes/node-e2e/tasks",
            Some(json!({"session_id": "agent-ghost-9", "prompt": "x"})),
        )
        .await;
    assert_eq!(status, 409, "{body}");

    // A drained control plane refuses brand-new tasks.
    let (status, _) = h.req(Method::POST, "/api/admin/drain", None).await;
    assert_eq!(status, 200);
    let (status, body) = h
        .req(
            Method::POST,
            "/api/nodes/node-e2e/tasks",
            Some(json!({"id": "agent-drained-1", "prompt": "x"})),
        )
        .await;
    assert_eq!(status, 503, "{body}");
}
