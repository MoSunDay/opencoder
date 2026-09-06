//! Session surface: explicit create/list plus the relay fallback that
//! forwards every /api/sessions/:id/* verb to the owning node as an
//! `http` command, SSE event streaming with cursor resume, and the
//! control-plane-owned task lookup.

use opencoder_core::fleet::{ExecutionKind, ExecutionStatus};
use reqwest::Method;
use serde_json::json;

use crate::support::{Harness, TOKEN};

#[tokio::test]
async fn sessions_create_and_list_through_node_summaries() {
    let h = Harness::new().await;
    let (status, body) = h
        .req(Method::POST, "/api/sessions", Some(json!({"agent": "act"})))
        .await;
    assert_eq!(status, 200, "{body}");
    let id = body["id"].as_str().unwrap().to_string();
    assert!(id.starts_with("agent-"), "{id}");
    assert_eq!(body["execution"]["kind"], json!("agent"));
    assert_eq!(body["execution"]["node_id"], json!("node-e2e"));

    // List merges the durable index with the node's `summary` command.
    h.node.set_command(
        &id,
        "summary",
        200,
        json!({"id": id, "title": "e2e session", "status": "idle"}),
    );
    let (status, body) = h.req(Method::GET, "/api/sessions", None).await;
    assert_eq!(status, 200, "{body}");
    let sessions = body["sessions"].as_array().unwrap();
    let mine = sessions.iter().find(|s| s["id"] == json!(id)).unwrap();
    assert_eq!(mine["title"], json!("e2e session"));
    assert_eq!(mine["node_id"], json!("node-e2e"));
}

#[tokio::test]
async fn relay_forwards_verbs_as_http_commands_and_validates_paths() {
    let h = Harness::new().await;
    h.put_index("agent-relay-1", ExecutionKind::Agent, ExecutionStatus::Idle)
        .await;
    h.node
        .set_command("agent-relay-1", "http", 200, json!({"answer": "from-node"}));
    let (status, body) = h
        .req(
            Method::POST,
            "/api/sessions/agent-relay-1/prompt?delivery=queue",
            Some(json!({"text": "hello"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["answer"], json!("from-node"));

    let seen = h.node.seen_commands();
    let http = seen
        .iter()
        .find(|(id, action, _)| id == "agent-relay-1" && action == "http")
        .expect("http command relayed");
    assert_eq!(http.2["method"], json!("POST"));
    assert_eq!(http.2["tail"], json!("prompt?delivery=queue"));
    assert_eq!(http.2["body"]["text"], json!("hello"));

    // GET relay with empty tail and no body.
    let (status, body) = h
        .req(Method::GET, "/api/sessions/agent-relay-1", None)
        .await;
    assert_eq!(status, 200, "{body}");
    // Path traversal and malformed ids never reach a node.
    let (status, body) = h
        .req(Method::GET, "/api/sessions/..%2Fetc%2Fpasswd", None)
        .await;
    assert_ne!(status, 200, "{body}");
    let resp = h
        .req_raw(Method::GET, "/api/sessions/bad id/prompt", None, None)
        .await;
    assert_eq!(resp.status(), 401, "auth applies before routing");
    let (status, body) = h
        .req(Method::GET, "/api/sessions/agent-unknown-9/prompt", None)
        .await;
    assert_eq!(status, 404, "{body}");
}

#[tokio::test]
async fn session_events_stream_replays_and_resumes_by_cursor() {
    let h = Harness::new().await;
    h.put_index("agent-sse-1", ExecutionKind::Agent, ExecutionStatus::Idle)
        .await;
    h.node.set_events(
        "agent-sse-1",
        vec![
            json!({"seq": 1, "kind": "llm_round_start", "data": {"n": 1}, "ts": 1}),
            json!({"seq": 2, "kind": "text_delta", "data": {"t": "hi"}, "ts": 2}),
            json!({"seq": 3, "kind": "done", "data": {}, "ts": 3}),
        ],
        true,
    );
    let (status, text) = h.sse_text("/api/sessions/agent-sse-1/events").await;
    assert_eq!(status, 200);
    assert!(text.contains("id: 1"), "{text}");
    assert!(text.contains("event: llm_round_start"), "{text}");
    assert!(text.contains("event: text_delta"), "{text}");
    assert!(text.contains("event: done"), "{text}");

    // Cursor resume: after=2 only replays seq 3.
    let (status, text) = h.sse_text("/api/sessions/agent-sse-1/events?after=2").await;
    assert_eq!(status, 200);
    assert!(!text.contains("id: 1") && !text.contains("id: 2"), "{text}");
    assert!(text.contains("id: 3"), "{text}");

    // Unknown execution: plain JSON 404, not an SSE stream.
    let (status, body) = h
        .req(Method::GET, "/api/sessions/agent-none/events", None)
        .await;
    assert_eq!(status, 404, "{body}");
}

#[tokio::test]
async fn task_owner_lookup_is_control_plane_owned() {
    let h = Harness::new().await;
    h.put_index(
        "agent-owner-1",
        ExecutionKind::Agent,
        ExecutionStatus::Running,
    )
    .await;
    let (status, body) = h
        .req(Method::GET, "/api/sessions/agent-owner-1/task", None)
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["task"]["task_id"], json!("agent-owner-1"));
    assert_eq!(body["task"]["session_id"], json!("agent-owner-1"));
    assert_eq!(body["node_id"], json!("node-e2e"));
    assert_eq!(body["task"]["status"], json!("running"));
    let (status, body) = h
        .req(Method::GET, "/api/sessions/agent-none/task", None)
        .await;
    assert_eq!(status, 404, "{body}");
}

#[tokio::test]
async fn execution_events_stream_on_the_executions_surface() {
    let h = Harness::new().await;
    h.put_index("agent-exev-1", ExecutionKind::Agent, ExecutionStatus::Idle)
        .await;
    h.node.set_events(
        "agent-exev-1",
        vec![
            json!({"seq": 1, "kind": "llm_round_start", "data": {"n": 1}, "ts": 1}),
            json!({"seq": 2, "kind": "done", "data": {}, "ts": 2}),
        ],
        true,
    );
    let (status, text) = h.sse_text("/api/executions/agent-exev-1/events").await;
    assert_eq!(status, 200);
    assert!(
        text.contains("id: 1") && text.contains("event: llm_round_start"),
        "{text}"
    );
    assert!(
        text.contains("id: 2") && text.contains("event: done"),
        "{text}"
    );

    // Cursor resume through the executions alias: after=1 only replays seq 2.
    let (status, text) = h
        .sse_text("/api/executions/agent-exev-1/events?after=1")
        .await;
    assert_eq!(status, 200);
    assert!(!text.contains("id: 1") && text.contains("id: 2"), "{text}");

    // Unknown execution: plain JSON 404, not an SSE stream.
    let (status, body) = h
        .req(Method::GET, "/api/executions/agent-none/events", None)
        .await;
    assert_eq!(status, 404, "{body}");
}

#[tokio::test]
async fn sessions_create_honors_caller_ids_pins_and_validates() {
    let h = Harness::new().await;
    // Caller-supplied id is honored verbatim in the receipt.
    let (status, body) = h
        .req(
            Method::POST,
            "/api/sessions",
            Some(json!({"id": "agent-named-1", "agent": "act"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["id"], json!("agent-named-1"));
    assert_eq!(body["execution"]["id"], json!("agent-named-1"));
    assert_eq!(body["execution"]["node_id"], json!("node-e2e"));

    // Pinning table: connected node works, ghost node finds no eligible
    // fleet member, malformed node ids are refused by validation.
    for (name, id, node, expected) in [
        ("pin connected", "agent-pin-1", Some("node-e2e"), 200),
        ("pin ghost", "agent-pin-2", Some("ghost"), 503),
        ("malformed node id", "agent-pin-3", Some("bad/id"), 400),
    ] {
        let mut body = json!({"id": id, "agent": "act"});
        if let Some(node) = node {
            body["node_id"] = json!(node);
        }
        let (status, body) = h.req(Method::POST, "/api/sessions", Some(body)).await;
        assert_eq!(status, expected, "{name}: {body}");
    }

    let (status, body) = h
        .req(Method::POST, "/api/sessions", Some(json!({"id": "bad id"})))
        .await;
    assert_eq!(status, 400, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("id must start with agent-"),
        "{body}"
    );
}

#[tokio::test]
async fn frozen_gate_refuses_session_create_but_not_reads() {
    let h = Harness::new().await;
    let (status, _) = h.req(Method::POST, "/api/admin/drain", None).await;
    assert_eq!(status, 200);
    let (status, body) = h
        .req(Method::POST, "/api/sessions", Some(json!({"agent": "act"})))
        .await;
    assert_eq!(status, 503, "{body}");
    // Listing is a pure index read and stays available while frozen.
    let (status, body) = h.req(Method::GET, "/api/sessions", None).await;
    assert_eq!(status, 200, "{body}");
}

#[tokio::test]
async fn session_list_falls_back_to_degraded_rows_without_summaries() {
    let h = Harness::new().await;
    h.put_index("agent-deg-1", ExecutionKind::Agent, ExecutionStatus::Idle)
        .await;
    let (status, body) = h.req(Method::GET, "/api/sessions", None).await;
    assert_eq!(status, 200, "{body}");
    let row = body["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == json!("agent-deg-1"))
        .expect("degraded row present");
    assert_eq!(row["node_id"], json!("node-e2e"));
    assert_eq!(row["status"], json!("idle"));
    assert!(row["created_at"].as_i64().unwrap_or(0) > 0, "{row}");
    // The failed `summary` reply body is preserved verbatim as detail_error.
    assert_eq!(
        row["detail_error"]["error"],
        json!("unknown execution command")
    );
}

#[tokio::test]
async fn relay_rejects_bad_ids_tails_and_bodies_before_any_node_call() {
    let h = Harness::new().await;
    h.put_index("agent-relay-2", ExecutionKind::Agent, ExecutionStatus::Idle)
        .await;
    // Malformed ids are refused by the path guard; '_' is a legal id char,
    // so `bad_id` passes the guard and lands on the 404 index miss instead.
    let (status, body) = h.req(Method::GET, "/api/sessions/bad.id", None).await;
    assert_eq!(status, 400, "{body}");
    let (status, body) = h.req(Method::GET, "/api/sessions/bad_id", None).await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["error"], json!("execution id not found"));
    // Traversal-style tails never reach a node.
    let (status, body) = h
        .req(Method::GET, "/api/sessions/agent-relay-2/a..b", None)
        .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(h.node.seen_commands().len(), 0, "no node roundtrip yet");

    // Non-JSON bodies are refused by the relay extraction regardless of the
    // declared content type (the body itself fails to parse).
    let (status, bytes) = h
        .req_bytes(
            Method::POST,
            "/api/sessions/agent-relay-2/prompt",
            Some("hi".into()),
            Some("text/plain"),
            Some(TOKEN),
            &[],
        )
        .await;
    assert_eq!(status, 400, "{}", String::from_utf8_lossy(&bytes));
    // Bodies over the 2 MiB frame limit are refused the same way.
    let oversized = format!("\"{}\"", "a".repeat(2 * 1024 * 1024 + 1));
    let (status, _) = h
        .req_bytes(
            Method::POST,
            "/api/sessions/agent-relay-2/prompt",
            Some(oversized),
            Some("application/json"),
            Some(TOKEN),
            &[],
        )
        .await;
    assert_eq!(status, 400);
    assert_eq!(h.node.seen_commands().len(), 0, "still no node roundtrip");
}

#[tokio::test]
async fn relay_forwards_put_delete_and_node_error_bodies_verbatim() {
    let h = Harness::new().await;
    h.put_index("agent-relay-3", ExecutionKind::Agent, ExecutionStatus::Idle)
        .await;
    h.node
        .set_command("agent-relay-3", "http", 200, json!({"ok": true}));
    let (status, body) = h
        .req(
            Method::PUT,
            "/api/sessions/agent-relay-3/answer",
            Some(json!({"x": 1})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = h
        .req(Method::DELETE, "/api/sessions/agent-relay-3/answer", None)
        .await;
    assert_eq!(status, 200, "{body}");
    let seen = h.node.seen_commands();
    for (method, body) in [
        ("PUT", json!({"x": 1})),
        ("DELETE", serde_json::Value::Null),
    ] {
        let call = seen
            .iter()
            .find(|(id, action, input)| {
                id == "agent-relay-3" && action == "http" && input["method"] == json!(method)
            })
            .unwrap_or_else(|| panic!("{method} not relayed: {seen:?}"));
        assert_eq!(call.2["tail"], json!("answer"), "{method}: {call:?}");
        assert_eq!(call.2["body"], body, "{method}: {call:?}");
    }

    // Node-side failures pass through with status and body intact.
    h.node.set_command(
        "agent-relay-3",
        "http",
        404,
        json!({"error": "no subresource"}),
    );
    let (status, body) = h
        .req(Method::GET, "/api/sessions/agent-relay-3/missing", None)
        .await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body, json!({"error": "no subresource"}));
}

#[tokio::test]
async fn admission_gate_blocks_only_mutating_session_relay_tails() {
    let h = Harness::new().await;
    h.put_index("agent-gate-1", ExecutionKind::Agent, ExecutionStatus::Idle)
        .await;
    h.node
        .set_command("agent-gate-1", "http", 200, json!({"ok": true}));
    let (status, _) = h.req(Method::POST, "/api/admin/drain", None).await;
    assert_eq!(status, 200);

    for tail in [
        "prompt",
        "fork",
        "compact",
        "handoff",
        "subagents/child/steer",
    ] {
        let (status, body) = h
            .req(
                Method::POST,
                &format!("/api/sessions/agent-gate-1/{tail}"),
                Some(json!({"text": "hi"})),
            )
            .await;
        assert_eq!(status, 503, "{tail}: {body}");
    }
    // Read-only and control tails stay reachable while frozen.
    let (status, body) = h
        .req(Method::GET, "/api/sessions/agent-gate-1/messages", None)
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = h
        .req(Method::POST, "/api/sessions/agent-gate-1/interrupt", None)
        .await;
    assert_eq!(status, 200, "{body}");

    // Reopen restores the mutating tails.
    let (status, _) = h.req(Method::DELETE, "/api/admin/drain", None).await;
    assert_eq!(status, 200);
    let (status, body) = h
        .req(
            Method::POST,
            "/api/sessions/agent-gate-1/prompt",
            Some(json!({"text": "hi"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
}

#[tokio::test]
async fn session_events_resume_from_last_event_id_header() {
    let h = Harness::new().await;
    h.put_index("agent-sse-2", ExecutionKind::Agent, ExecutionStatus::Idle)
        .await;
    h.node.set_events(
        "agent-sse-2",
        vec![
            json!({"seq": 1, "kind": "llm_round_start", "data": {}, "ts": 1}),
            json!({"seq": 2, "kind": "text_delta", "data": {"t": "hi"}, "ts": 2}),
            json!({"seq": 3, "kind": "done", "data": {}, "ts": 3}),
        ],
        true,
    );
    let (status, bytes) = h
        .req_bytes(
            Method::GET,
            "/api/sessions/agent-sse-2/events",
            None,
            None,
            Some(TOKEN),
            &[("last-event-id", "2")],
        )
        .await;
    let text = String::from_utf8(bytes).unwrap();
    assert_eq!(status, 200);
    assert!(!text.contains("id: 1") && !text.contains("id: 2"), "{text}");
    assert!(
        text.contains("id: 3") && text.contains("event: done"),
        "{text}"
    );
}
