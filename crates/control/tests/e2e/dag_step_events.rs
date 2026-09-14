//! DAG single-step event stream over the compat surface:
//! `GET /api/dag/runs/:id/steps/:step/events` is an SSE endpoint backed by the
//! `DagStepEvents` node RPC. These tests pin the routing, the cursor contract
//! (`?after=` and the `Last-Event-ID` reconnect fallback), first-page error
//! passthrough, the `more`/`finished` paging loop and the role gate.

use opencoder_core::fleet::{ExecutionKind, ExecutionStatus};
use reqwest::Method;
use serde_json::{json, Value};
use std::time::Duration;

use crate::support::{Harness, TOKEN};

/// One wasm-step frame in the locked node contract shape.
fn frame(seq: i64, text: &str) -> Value {
    json!({"seq": seq, "kind": "step_output",
           "data": {"step": "fetch", "stream": "stdout", "text": text, "at_ms": seq},
           "ts": seq})
}

async fn seeded(h: &Harness, id: &str) {
    h.put_index(id, ExecutionKind::Dag, ExecutionStatus::Running)
        .await;
}

#[tokio::test]
async fn dag_step_events_replay_the_step_stream_and_close_on_finished() {
    let h = Harness::new().await;
    seeded(&h, "dag-se-1").await;
    h.node.set_step_events(
        "dag-se-1",
        "fetch",
        vec![
            frame(1, "hello"),
            frame(2, "world"),
            json!({"seq": 3, "kind": "step_finished",
                   "data": {"status": "done", "error": null,
                            "started_at_ms": 10, "finished_at_ms": 20},
                   "ts": 21}),
        ],
        true,
    );
    let (status, text) = h
        .sse_text("/api/dag/runs/dag-se-1/steps/fetch/events?after=0")
        .await;
    assert_eq!(status, 200);
    assert!(text.contains("event: step_output"), "{text}");
    assert!(text.contains("event: step_finished"), "{text}");
    assert!(
        text.contains("id: 1\n") && text.contains("id: 3\n"),
        "{text}"
    );
    assert!(text.contains("hello") && text.contains("world"), "{text}");
}

/// The event cursor is authoritative on both channels: the query parameter and
/// the browser's reconnect header.
#[tokio::test]
async fn dag_step_events_resume_from_after_and_last_event_id() {
    let h = Harness::new().await;
    seeded(&h, "dag-se-2").await;
    h.node.set_step_events(
        "dag-se-2",
        "fetch",
        vec![frame(1, "one"), frame(2, "two"), frame(3, "three")],
        true,
    );
    let (status, text) = h
        .sse_text("/api/dag/runs/dag-se-2/steps/fetch/events?after=2")
        .await;
    assert_eq!(status, 200);
    assert!(text.contains("id: 3\n"), "{text}");
    assert!(
        !text.contains("id: 1\n") && !text.contains("id: 2\n"),
        "{text}"
    );

    let (status, bytes) = h
        .req_bytes(
            Method::GET,
            "/api/dag/runs/dag-se-2/steps/fetch/events",
            None,
            None,
            Some(TOKEN),
            &[("last-event-id", "2")],
        )
        .await;
    assert_eq!(status, 200);
    let text = String::from_utf8(bytes).unwrap();
    assert!(
        text.contains("id: 3\n") && !text.contains("id: 1\n"),
        "{text}"
    );
}

/// `more` outranks `finished`: the loop keeps polling (and never closes) until
/// the node drains the page, exactly like the run-level event stream.
#[tokio::test]
async fn dag_step_events_more_flag_keeps_the_stream_polling() {
    let h = Harness::new().await;
    seeded(&h, "dag-se-3").await;
    h.node
        .set_step_events_more("dag-se-3", "fetch", vec![frame(1, "one")], true, true);
    let streamed = tokio::time::timeout(
        Duration::from_millis(900),
        h.sse_text("/api/dag/runs/dag-se-3/steps/fetch/events?after=0"),
    )
    .await;
    assert!(
        streamed.is_err(),
        "more=true must keep the stream open: {streamed:?}"
    );
}

/// Errors surface as the HTTP status of the first page: an unindexed run never
/// reaches a node, and node-side validation (404 unknown step, 400 non-DAG)
/// passes through verbatim instead of degrading into an SSE error frame.
#[tokio::test]
async fn dag_step_events_first_page_errors_pass_through() {
    let h = Harness::new().await;
    let (status, body) = h
        .req(
            Method::GET,
            "/api/dag/runs/dag-se-none/steps/fetch/events",
            None,
        )
        .await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["error"], json!("execution id not found"));

    seeded(&h, "dag-se-4").await;
    for (step, code, message) in [
        ("missing", 404, "step not found in run spec"),
        ("fetch", 400, "dag step events require a DAG execution"),
    ] {
        h.node
            .set_step_events_status("dag-se-4", step, code, json!({"error": message}));
        let (status, body) = h
            .req(
                Method::GET,
                &format!("/api/dag/runs/dag-se-4/steps/{step}/events"),
                None,
            )
            .await;
        assert_eq!(status, code, "{body}");
        assert_eq!(body["error"], json!(message), "{body}");
    }
}

/// The compat DAG surface stays admin-only under the role gate; the step event
/// stream inherits that (see `role_gate::allowed`).
#[tokio::test]
async fn dag_step_events_are_admin_only() {
    let h = Harness::new().await;
    seeded(&h, "dag-se-5").await;
    h.node
        .set_step_events("dag-se-5", "fetch", vec![frame(1, "one")], true);
    let (status, body) = h
        .req(
            Method::POST,
            "/api/users",
            Some(json!({"name": "dag-se-user", "role": "user"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let token = body["token"].as_str().unwrap();
    let denied = h
        .req_raw(
            Method::GET,
            "/api/dag/runs/dag-se-5/steps/fetch/events",
            None,
            Some(token),
        )
        .await;
    assert_eq!(denied.status(), 403);
    let allowed = h
        .req_raw(
            Method::GET,
            "/api/dag/runs/dag-se-5/steps/fetch/events",
            None,
            Some(TOKEN),
        )
        .await;
    assert_eq!(allowed.status(), 200);
}
