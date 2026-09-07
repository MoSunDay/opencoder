//! DAG run lifecycle through the control plane: dispatch from a saved
//! definition, run ledger views, SSE events and cancel.

use opencoder_core::fleet::{ExecutionKind, ExecutionStatus};
use reqwest::Method;
use serde_json::json;

use crate::support::Harness;

const SPEC: &str = r#"{"name":"etl-demo","steps":[
    {"name":"fetch","kind":{"type":"wasm","command":"tool.wasm"}},
    {"name":"load","depends_on":["fetch"],"kind":{"type":"wasm","command":"tool.wasm"}}]}"#;

async fn seed_definition(h: &Harness) {
    let spec: serde_json::Value = serde_json::from_str(SPEC).unwrap();
    let (status, body) = h
        .req(Method::POST, "/api/dag/defs", Some(json!({"spec": spec})))
        .await;
    assert_eq!(status, 200, "{body}");
}

#[tokio::test]
async fn dispatch_creates_run_and_ledger_views_route_to_the_node() {
    let h = Harness::new().await;
    seed_definition(&h).await;
    let (status, body) = h
        .req(
            Method::POST,
            "/api/dag/defs/etl-demo/dispatch",
            Some(json!({"id": "dag-run-1"})),
        )
        .await;
    assert_eq!(status, 202, "{body}");
    assert_eq!(body["run_id"], json!("dag-run-1"));
    assert_eq!(body["execution"]["kind"], json!("dag"));
    assert_eq!(body["execution"]["node_id"], json!("node-e2e"));
    assert_eq!(body["execution"]["status"], json!("pending"));

    // Ledger view merges the durable index with the node's inspect payload.
    h.node.set_inspect(
        "dag-run-1",
        json!({"execution": {"id": "dag-run-1", "status": "done"},
               "request": {"target": "etl-demo"},
               "definition": {"spec": {"name": "etl-demo", "steps": [
                   {"name": "fetch", "kind": {"type":"wasm","command":"tool.wasm"}},
                   {"name": "load", "depends_on": ["fetch"], "kind": {"type":"wasm","command":"tool.wasm"}}]}},
               "result": {"steps": {"fetch": "ok"}}}),
    );
    let (status, body) = h.req(Method::GET, "/api/dag/runs", None).await;
    assert_eq!(status, 200, "{body}");
    let runs = body.as_array().unwrap();
    let mine = runs.iter().find(|r| r["id"] == json!("dag-run-1")).unwrap();
    assert_eq!(mine["dag_id"], json!("etl-demo"));

    let (status, body) = h.req(Method::GET, "/api/dag/runs/dag-run-1", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["dag_id"], json!("etl-demo"), "{body}");
    assert!(
        body["spec"]
            .as_object()
            .is_some_and(|s| s.contains_key("steps")),
        "{body}"
    );
    let (status, body) = h.req(Method::GET, "/api/dag/runs/dag-none", None).await;
    assert_eq!(status, 404, "{body}");

    // Dispatch of an unknown definition is a clean 404 (target resolution).
    let (status, body) = h
        .req(
            Method::POST,
            "/api/dag/defs/none/dispatch",
            Some(json!({"id": "dag-run-x"})),
        )
        .await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["error"], json!("definition not found"));
}

#[tokio::test]
async fn dag_run_events_stream_and_cancel() {
    let h = Harness::new().await;
    seed_definition(&h).await;
    let (status, _) = h
        .req(
            Method::POST,
            "/api/dag/defs/etl-demo/dispatch",
            Some(json!({"id": "dag-run-2"})),
        )
        .await;
    assert_eq!(status, 202);

    h.node.set_events(
        "dag-run-2",
        vec![
            json!({"seq": 1, "kind": "step_started", "data": {"step": "fetch"}, "ts": 1}),
            json!({"seq": 2, "kind": "step_finished", "data": {"step": "fetch"}, "ts": 2}),
        ],
        true,
    );
    let (status, text) = h.sse_text("/api/dag/runs/dag-run-2/events?after=0").await;
    assert_eq!(status, 200);
    assert!(
        text.contains("event: step_started") && text.contains("event: step_finished"),
        "{text}"
    );

    h.node.set_command(
        "dag-run-2",
        "cancel",
        200,
        json!({"id": "dag-run-2", "status": "cancelled"}),
    );
    let (status, body) = h
        .req(Method::POST, "/api/dag/runs/dag-run-2/cancel", None)
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["status"], json!("cancelled"));
    // Cancel of an unindexed run never reaches a node.
    let (status, body) = h
        .req(Method::POST, "/api/dag/runs/dag-none/cancel", None)
        .await;
    assert_eq!(status, 404, "{body}");
}

#[tokio::test]
async fn artifact_downloads_through_the_dag_run_surface() {
    let h = Harness::new().await;
    h.put_index("dag-art-2", ExecutionKind::Dag, ExecutionStatus::Done)
        .await;
    h.node
        .set_artifact("dag-art-2", "fetch", "output.txt", b"42\n".to_vec());
    let resp = h
        .req_raw(
            Method::GET,
            "/api/executions/dag-art-2/artifact?step=fetch",
            None,
            Some(crate::support::TOKEN),
        )
        .await;
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.bytes().await.unwrap(), "42\n");
}

/// A run whose owning node cannot be inspected (offline node, dropped run)
/// must still list: the durable index plus `detail_error` instead of the
/// `dag_view` merge, so clients can tell the two shapes apart.
#[tokio::test]
async fn dag_list_falls_back_to_the_index_when_inspect_fails() {
    let h = Harness::new().await;
    // No inspect seed: the node answers 404 for this execution.
    h.put_index("dag-f1", ExecutionKind::Dag, ExecutionStatus::Running)
        .await;
    let (status, body) = h.req(Method::GET, "/api/dag/runs", None).await;
    assert_eq!(status, 200, "{body}");
    let runs = body.as_array().unwrap();
    assert_eq!(runs.len(), 1, "{body}");
    let row = &runs[0];
    assert_eq!(row["id"], json!("dag-f1"));
    assert_eq!(row["kind"], json!("dag"));
    assert_eq!(row["execution_status"], json!("running"));
    assert_eq!(row["execution_created_at"], row["created_at"]);
    assert_eq!(row["detail_error"], json!({"error": "execution not found"}));
    assert!(row.get("spec").is_none(), "{body}");
    assert!(row.get("dag_id").is_none(), "{body}");
}

/// The event cursor is authoritative: `?after=1` replays only seq>=2 frames.
#[tokio::test]
async fn dag_run_events_resume_from_the_after_cursor() {
    let h = Harness::new().await;
    h.put_index("dag-cur-1", ExecutionKind::Dag, ExecutionStatus::Running)
        .await;
    h.node.set_events(
        "dag-cur-1",
        vec![
            json!({"seq": 1, "kind": "step_started", "data": {"step": "a"}, "ts": 1}),
            json!({"seq": 2, "kind": "step_finished", "data": {"step": "a"}, "ts": 2}),
            json!({"seq": 3, "kind": "run_done", "data": {}, "ts": 3}),
        ],
        true,
    );
    let (status, text) = h.sse_text("/api/dag/runs/dag-cur-1/events?after=1").await;
    assert_eq!(status, 200);
    assert!(
        text.contains("id: 2\n") && text.contains("id: 3\n"),
        "{text}"
    );
    assert!(
        text.contains("event: step_finished") && text.contains("event: run_done"),
        "{text}"
    );
    assert!(!text.contains("id: 1\n"), "{text}");
    assert!(!text.contains("event: step_started"), "{text}");
}

/// Controls and views of an unknown workflow id never reach a node: the
/// durable index is the routing authority, so all three surface a clean 404.
#[tokio::test]
async fn todo_workflow_controls_and_view_of_unknown_id_are_404() {
    let h = Harness::new().await;
    for (label, method, path) in [
        ("view", Method::GET, "/api/todo/workflows/todos-none"),
        (
            "interrupt",
            Method::POST,
            "/api/todo/workflows/todos-none/interrupt",
        ),
        (
            "resume",
            Method::POST,
            "/api/todo/workflows/todos-none/resume",
        ),
    ] {
        let (status, body) = h.req(method, path, None).await;
        assert_eq!(status, 404, "{label}: {body}");
        assert_eq!(
            body["error"],
            json!("execution id not found"),
            "{label}: {body}"
        );
    }
    assert!(h.node.seen_commands().is_empty());
}
