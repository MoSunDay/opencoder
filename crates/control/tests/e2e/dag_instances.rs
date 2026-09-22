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

#[tokio::test]
async fn old_node_cannot_accept_dynamic_dispatch_or_instance_queries() {
    use opencoder_core::fleet::RpcReply;
    let h = Harness::new().await;
    h.node
        .set_capability_reply(RpcReply::ok(json!({"compatible":true})));
    let spec = json!({"name":"mixed-dynamic","steps":[{"name":"batch","kind":{
        "type":"dynamic", "source":{"type":"input","pointer":"/items"},
        "template":{"type":"agent","prompt":"process item"}
    }}]});
    let (code, body) = h
        .req(
            Method::POST,
            "/api/executions",
            Some(json!({
                "id":"dag-incompatible", "kind":"dag", "target":"mixed-dynamic",
                "node_id":h.node.id, "input":{"definition":spec,"items":["one"]}
            })),
        )
        .await;
    assert_eq!(code, 503, "{body}");
    assert!(body.to_string().contains("dag_dynamic_v1"));
    assert!(h.node.journal_ids().is_empty());
    assert!(h
        .state
        .fleet
        .index("dag-incompatible")
        .await
        .unwrap()
        .is_none());
    assert!(h
        .state
        .fleet
        .assignment("dag-incompatible")
        .await
        .unwrap()
        .is_none());
    h.put_index("dag-old", ExecutionKind::Dag, ExecutionStatus::Done)
        .await;
    for tail in [
        "/api/dag/runs/dag-old/steps/batch/instances",
        "/api/dag/runs/dag-old/steps/batch/instances/0",
        "/api/dag/runs/dag-old/steps/batch/instances/0/events",
        "/api/executions/dag-old/artifact?step=batch&index=0&file=output.txt",
    ] {
        let (code, body) = h.req(Method::GET, tail, None).await;
        assert_eq!(code, 409, "{tail}: {body}");
        assert!(body.to_string().contains("dag_dynamic_v1"));
    }
    // Existing static DAGs continue using the unchanged protocol.
    let (code, body) = h
        .req(
            Method::POST,
            "/api/executions",
            Some(json!({
                "id":"dag-static-old", "kind":"dag", "target":"static",
                "input":{"definition":{"name":"static","steps":[{"name":"work","kind":{
                    "type":"agent", "prompt":"work"
                }}]}}
            })),
        )
        .await;
    assert_eq!(code, 202, "{body}");
}

#[tokio::test]
async fn automatic_dynamic_placement_skips_old_nodes() {
    use opencoder_core::fleet::{NodeAdmissionCommand, NodeOperation, RpcReply};
    use opencoder_node::fleet::NodeService;
    use std::{sync::Arc, time::Duration};
    let h = Harness::new().await;
    h.node
        .set_capability_reply(RpcReply::ok(json!({"compatible":true})));
    let compatible = crate::support::MockNode::new("node-new");
    let service: Arc<dyn NodeService> = compatible.clone();
    let base = h.base.clone();
    let link = tokio::spawn(async move { opencoder_node::fleet::run(&base, TOKEN, service).await });
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if h.state.hub.views().await.iter().any(|n| {
                n.registration.id == "node-new"
                    && n.online
                    && n.snapshot.as_ref().is_some_and(|s| s.ready)
            }) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let (code, body) = h
        .req(
            Method::POST,
            "/api/executions",
            Some(json!({
                "id":"dag-compatible", "kind":"dag", "target":"dynamic",
                "input":{"definition":{"name":"dynamic","steps":[{"name":"batch","kind":{
                    "type":"dynamic", "source":{"type":"input","pointer":"/items"},
                    "template":{"type":"agent","prompt":"process"}
                }}]}, "items":["one"]}
            })),
        )
        .await;
    assert_eq!(code, 202, "{body}");
    assert_eq!(body["node_id"], "node-new");
    assert!(h.node.journal_ids().is_empty());
    assert_eq!(compatible.journal_ids(), vec!["dag-compatible"]);
    // The old node remains connected after negotiation.
    assert_eq!(
        h.state
            .hub
            .call(
                &h.node.id,
                NodeOperation::Admission {
                    command: NodeAdmissionCommand::Status
                }
            )
            .await
            .status,
        200
    );
    link.abort();
}

#[tokio::test]
async fn old_nodes_are_excluded_from_layered_children_before_assignment() {
    use opencoder_core::fleet::RpcReply;
    let h = Harness::new().await;
    h.node
        .set_capability_reply(RpcReply::ok(json!({"compatible":true})));
    let (code, body) = h
        .req(
            Method::POST,
            "/api/executions",
            Some(json!({
                "id":"agent-brain-incompatible", "kind":"agent", "target":"act",
                "input":{"schema_version":4,"brain_layered":{"run_id":"root"},"prompt":"bounded task"}
            })),
        )
        .await;
    assert_eq!(code, 503, "{body}");
    assert!(body.to_string().contains("brain_scheduler_v4"));
    assert!(h
        .state
        .fleet
        .assignment("agent-brain-incompatible")
        .await
        .unwrap()
        .is_none());
    assert!(h.node.journal_ids().is_empty());
    let (code, body) = h.req(Method::POST, "/api/executions", Some(json!({
        "id":"agent-ordinary", "kind":"agent", "target":"act", "input":{"prompt":"ordinary task"}
    }))).await;
    assert_eq!(code, 202, "{body}");
}
