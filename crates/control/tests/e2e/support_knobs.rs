//! Coverage for the scripting knobs on the e2e harness itself: raw Create /
//! Events / Admission / Artifact overrides, snapshot field overrides,
//! journal accessors and the generic raw HTTP helper.

use std::time::Duration;

use opencoder_core::fleet::{ExecutionKind, ExecutionStatus};
use reqwest::Method;
use serde_json::json;

use crate::support::{Harness, TOKEN};

/// Runs one node RPC so the WS client publishes a fresh snapshot.
async fn bump_snapshot(h: &Harness) {
    h.node.set_maintenance("status", 200, json!({"ok": true}));
    let (status, body) = h
        .req(
            Method::POST,
            "/api/nodes/node-e2e/maintenance",
            Some(json!({"action": "status"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
}

#[tokio::test]
async fn create_reply_override_bypasses_the_journal() {
    let h = Harness::new().await;
    h.node
        .set_create_reply(409, json!({"error": "node refuses"}));
    let (status, body) = h
        .req(
            Method::POST,
            "/api/executions",
            Some(json!({"id": "agent-knob-1", "kind": "agent"})),
        )
        .await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["error"], json!("node refuses"));
    assert!(h.node.journal_ids().is_empty(), "override must not journal");

    // Clearing restores real journaling + idempotent acceptance.
    h.node.clear_create_reply();
    let (status, body) = h
        .req(
            Method::POST,
            "/api/executions",
            Some(json!({"id": "agent-knob-1", "kind": "agent"})),
        )
        .await;
    assert_eq!(status, 202, "{body}");
    assert_eq!(h.node.journal_ids(), vec!["agent-knob-1".to_string()]);
}

#[tokio::test]
async fn events_more_flag_controls_stream_termination() {
    let h = Harness::new().await;
    h.put_index("agent-knob-2", ExecutionKind::Agent, ExecutionStatus::Idle)
        .await;
    let rows = vec![
        json!({"seq": 1, "kind": "llm_round_start", "data": {}, "ts": 1}),
        json!({"seq": 2, "kind": "done", "data": {}, "ts": 2}),
    ];
    // more=false: the SSE stream ends after replay (also exercises
    // req_bytes with a verbatim last-event-id resume header).
    h.node
        .set_events_more("agent-knob-2", rows.clone(), true, false);
    let (status, bytes) = h
        .req_bytes(
            Method::GET,
            "/api/executions/agent-knob-2/events",
            None,
            None,
            Some(TOKEN),
            &[("last-event-id", "1")],
        )
        .await;
    let text = String::from_utf8(bytes).unwrap();
    assert_eq!(status, 200);
    assert!(!text.contains("id: 1"), "{text}");
    assert!(
        text.contains("id: 2") && text.contains("event: done"),
        "{text}"
    );

    // more=true: rows replay but the stream keeps polling forever, so the
    // raw read must not complete.
    h.node.set_events_more("agent-knob-2", rows, true, true);
    let pending = h.req_bytes(
        Method::GET,
        "/api/executions/agent-knob-2/events",
        None,
        None,
        Some(TOKEN),
        &[],
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(800), pending)
            .await
            .is_err(),
        "more=true must keep the SSE stream open"
    );
}

#[tokio::test]
async fn events_status_override_replies_raw_and_clears_back() {
    let h = Harness::new().await;
    h.put_index("agent-knob-3", ExecutionKind::Agent, ExecutionStatus::Idle)
        .await;
    h.node
        .set_events_status("agent-knob-3", 503, json!({"error": "node exploding"}));
    let (status, body) = h
        .req(Method::GET, "/api/executions/agent-knob-3/events", None)
        .await;
    assert_eq!(status, 503, "{body}");
    assert_eq!(body["error"], json!("node exploding"));

    // Seeding rows clears the raw override: the stream works again.
    h.node.set_events(
        "agent-knob-3",
        vec![json!({"seq": 1, "kind": "done", "data": {}, "ts": 1})],
        true,
    );
    let (status, text) = h.sse_text("/api/executions/agent-knob-3/events").await;
    assert_eq!(status, 200);
    assert!(text.contains("event: done"), "{text}");
}

#[tokio::test]
async fn admission_reply_override_skips_default_side_effects() {
    let h = Harness::new().await;
    h.node
        .set_admission_reply("freeze", 500, json!({"mode": "frozen", "active_runs": 7}));
    let (status, body) = h.req(Method::POST, "/api/admin/drain", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["nodes"][0]["status"], json!(500), "{body}");
    assert_eq!(body["nodes"][0]["body"]["active_runs"], json!(7), "{body}");
    assert_eq!(body["drained"], json!(false), "{body}");
    assert!(h.node.open.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(h.node.freezes_count(), 0, "override must not count freezes");
}

#[tokio::test]
async fn default_freeze_still_counts() {
    let h = Harness::new().await;
    let (status, _) = h.req(Method::POST, "/api/admin/drain", None).await;
    assert_eq!(status, 200);
    assert_eq!(h.node.freezes_count(), 1);
}

#[tokio::test]
async fn snapshot_opts_override_reaches_the_node_catalog() {
    let h = Harness::new().await;
    // Any node RPC pushes a fresh snapshot; maintenance is the cheapest.
    h.node.set_snapshot_opts(Some(3), None);
    bump_snapshot(&h).await;
    let (status, body) = h.req(Method::GET, "/api/nodes", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["nodes"][0]["snapshot"]["active_agent_loops"], json!(3));

    h.node.set_snapshot_opts(None, Some(false));
    bump_snapshot(&h).await;
    let (status, body) = h.req(Method::GET, "/api/nodes", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["nodes"][0]["snapshot"]["ready"], json!(false));
}

#[tokio::test]
async fn artifact_raw_replies_override_protocol_chunking() {
    let h = Harness::new().await;
    h.put_index("dag-knob-1", ExecutionKind::Dag, ExecutionStatus::Idle)
        .await;
    // Protocol-derived bytes for the same key must be bypassed.
    h.node
        .set_artifact("dag-knob-1", "build", "out.txt", b"protocol-bytes".to_vec());
    h.node.set_artifact_raw(
        "dag-knob-1",
        "build",
        "out.txt",
        vec![opencoder_core::fleet::RpcReply::ok(json!({
            "step": "build", "file": "out.txt",
            "offset": 0, "next_offset": 4, "total_bytes": 4,
            "version": "v1", "eof": true, "encoding": "base64",
            "bytes_b64": "cmF3IQ==",
        }))],
    );
    let (status, bytes) = h
        .req_bytes(
            Method::GET,
            "/api/executions/dag-knob-1/artifact?step=build&file=out.txt",
            None,
            None,
            Some(TOKEN),
            &[],
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(bytes, b"raw!".to_vec());
}

#[tokio::test]
async fn req_bytes_sends_raw_bodies_with_explicit_content_type() {
    let h = Harness::new().await;
    let (status, bytes) = h
        .req_bytes(
            Method::POST,
            "/api/teams",
            Some("{not json".into()),
            Some("application/json"),
            Some(TOKEN),
            &[],
        )
        .await;
    assert_eq!(status, 400, "{}", String::from_utf8_lossy(&bytes));

    // No token: bearer gate still applies to raw requests.
    let (status, _) = h
        .req_bytes(Method::GET, "/api/nodes", None, None, None, &[])
        .await;
    assert_eq!(status, 401);
}
