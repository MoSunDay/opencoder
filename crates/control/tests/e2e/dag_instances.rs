use crate::support::{Harness, TOKEN};
use opencoder_core::fleet::{ExecutionKind, ExecutionStatus};
use reqwest::Method;
use serde_json::json;

#[tokio::test]
async fn instance_routes_preserve_identity_and_bound_pages() {
    let h = Harness::new().await;
    h.put_index("dag-dynamic", ExecutionKind::Dag, ExecutionStatus::Running)
        .await;
    let base = "/api/dag/runs/dag-dynamic/steps/process/instances";
    let (code, body) = h.req(Method::GET, base, None).await;
    assert_eq!(code, 200, "{body}");
    assert_eq!(body["limit"], 100);
    assert_eq!(body["offset"], 0);
    let (code, body) = h
        .req(Method::GET, &format!("{base}?offset=900&limit=500"), None)
        .await;
    assert_eq!(code, 200);
    assert_eq!(body["limit"], 200);
    assert_eq!(body["offset"], 900);
    let (code, body) = h.req(Method::GET, &format!("{base}/42"), None).await;
    assert_eq!(code, 200);
    assert_eq!(body["index"], 42);
    assert_eq!(body["step"], "process");
    let (code, _) = h.req(Method::GET, &format!("{base}/-1"), None).await;
    assert_eq!(code, 400);
    let (code, _) = h
        .req(
            Method::GET,
            "/api/dag/runs/missing/steps/process/instances",
            None,
        )
        .await;
    assert_eq!(code, 404);
}

#[tokio::test]
async fn instance_sse_reconnects_at_cursor_and_returns_worker_errors() {
    let h = Harness::new().await;
    h.put_index("dag-dynamic", ExecutionKind::Dag, ExecutionStatus::Running)
        .await;
    h.node.set_step_events(
        "dag-dynamic",
        "process/instances/1",
        vec![
            json!({"seq":3,"kind":"step_output","data":{"index":1,"text":"past"},"ts":1}),
            json!({"seq":4,"kind":"step_output","data":{"index":1,"text":"current"},"ts":2}),
        ],
        true,
    );
    let base = "/api/dag/runs/dag-dynamic/steps/process/instances/1/events";
    let (code, bytes) = h
        .req_bytes(
            Method::GET,
            base,
            None,
            None,
            Some(TOKEN),
            &[("last-event-id", "3")],
        )
        .await;
    assert_eq!(code, 200);
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.contains("current"));
    assert!(!text.contains("past"));
    h.node.set_step_events_status(
        "dag-dynamic",
        "process/instances/1",
        404,
        json!({"error":"instance not found"}),
    );
    let (code, body) = h.req(Method::GET, base, None).await;
    assert_eq!(code, 404);
    assert_eq!(body["error"], "instance not found");
}
