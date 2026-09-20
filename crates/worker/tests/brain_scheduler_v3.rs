//! V3 root execution keeps a recoverable wake marker and never embeds child
//! execution output in the root journal.
mod support;

use opencoder_core::{brain::BrainSchedulerRequest, fleet::*};
use opencoder_llm::{LlmEvent, MockChatClient};
use opencoder_node::fleet::NodeService;
use serde_json::{json, Value};
use std::sync::Arc;
use support::*;

#[tokio::test]
async fn root_emits_one_scheduler_wake_until_control_acknowledges_it() {
    let (_config, _home) = isolated_config();
    let dir = tempfile::tempdir().unwrap();
    let node = worker(dir.path(), mock()).await;
    let id = "brain-v3-wake";
    let request = json!({
        "schema_version": 3,
        "objective": "review repository",
        "inputs": {"repo": "opencoder"},
        "max_rounds": 3
    });
    let reply = node
        .handle(NodeOperation::Create {
            assignment: assignment(
                &node,
                id,
                ExecutionKind::Brain,
                json!({"schema_version":3,"scheduler_request":request}),
                Some(json!({})),
            ),
        })
        .await;
    assert_eq!(reply.status, 200, "{reply:?}");
    settled(&node, id).await;
    let frames = node.brain_frames().await.unwrap();
    let (reference, generation) = frames
        .iter()
        .find_map(|frame| match frame {
            NodeFrame::Brain {
                execution,
                action,
                input,
            } if action == "scheduler_wake" => {
                Some((execution.clone(), input["generation"].clone()))
            }
            _ => None,
        })
        .expect("scheduler wake");
    assert_eq!(frames.iter().filter(|frame| matches!(frame, NodeFrame::Brain { action, .. } if action == "scheduler_wake")).count(), 1);
    node.handle(NodeOperation::Brain {
        execution: reference,
        action: "scheduler_wake_ack".into(),
        input: json!({"generation":generation}),
    })
    .await;
    assert!(node.brain_frames().await.unwrap().is_empty());
    node.shutdown().await.unwrap();
}

#[tokio::test]
async fn scheduler_context_requeues_idle_root_for_node_decision() {
    let (_config, _home) = isolated_config();
    let client = Arc::new(
        MockChatClient::new().with_default(vec![LlmEvent::Completed {
            text: r#"{"decision":"fail","reason":"test stop","error_type":"test"}"#.into(),
            tool_calls: vec![],
            usage: None,
        }]),
    );
    let dir = tempfile::tempdir().unwrap();
    let node = worker(dir.path(), client).await;
    let id = "brain-v3-context";
    let request = json!({
        "schema_version": 3,
        "objective": "exercise node activation",
        "inputs": {},
        "max_rounds": 1
    });
    let reference = ExecutionRef {
        id: id.into(),
        kind: ExecutionKind::Brain,
    };
    let reply = node
        .handle(NodeOperation::Create {
            assignment: assignment(
                &node,
                id,
                ExecutionKind::Brain,
                json!({"schema_version":3,"scheduler_request":request}),
                Some(json!({})),
            ),
        })
        .await;
    assert_eq!(reply.status, 200, "{reply:?}");
    settled(&node, id).await;
    let request: BrainSchedulerRequest = serde_json::from_value(request).unwrap();
    let snapshot = node
        .handle(NodeOperation::Brain {
            execution: reference.clone(),
            action: "snapshot".into(),
            input: json!({}),
        })
        .await;
    let context = json!({
        "schema_version": 3,
        "run_id": id,
        "generation": 0,
        "round": 0,
        "request": request,
        "capabilities": [],
        "operations": snapshot.body["operations"].clone(),
        "summaries": {}
    });
    assert_eq!(
        node.handle(NodeOperation::Brain {
            execution: reference,
            action: "scheduler_context".into(),
            input: context,
        })
        .await
        .status,
        200
    );
    let failed = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            let failed = node
                .handle(NodeOperation::Brain {
                    execution: ExecutionRef {
                        id: id.into(),
                        kind: ExecutionKind::Brain,
                    },
                    action: "snapshot".into(),
                    input: json!({}),
                })
                .await
                .body;
            assert_ne!(
                failed["run"]["phase"], "blocked",
                "node decision blocked: {failed}"
            );
            if failed["run"]["phase"] == "failed" {
                return failed;
            }
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        }
    })
    .await
    .expect("scheduler decision timeout");
    assert_eq!(failed["run"]["error"], "test");
    node.shutdown().await.unwrap();
}

#[path = "support/scheduler.rs"]
mod scheduler_support;
use scheduler_support::SchedulerClient;

#[tokio::test]
async fn control_round_trip_dispatches_child_and_waits_for_terminal_barrier() {
    let client = Arc::new(SchedulerClient::default());
    let fleet = Fleet::new(1, client).await;
    let id = "brain-v3-round-trip";
    let created = fleet
        .call(
            "POST",
            "/api/brain/runs",
            json!({
                "schema_version":3,
                "id":id,
                "objective":"inspect repository",
                "inputs":{"repo":"opencoder"},
                "max_rounds":2
            }),
        )
        .await;
    assert_eq!(created.status, 202, "{created:?}");
    let snapshot = scheduler_support::wait_phase(&fleet, id, "completed").await;
    let operations = snapshot["operations"].as_array().expect("operations");
    assert_eq!(operations.len(), 1, "{snapshot}");
    assert_eq!(operations[0]["status"], "done", "{snapshot}");
    let events = fleet
        .call(
            "GET",
            &format!("/api/brain/runs/{id}/events-page?after=0&limit=100"),
            Value::Null,
        )
        .await;
    assert_eq!(events.status, 200, "{events:?}");
    let event_types: Vec<_> = events.body["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|event| event["event_type"].as_str())
        .collect();
    assert!(event_types.contains(&"round_barrier_reached"), "{events:?}");
    assert!(event_types.contains(&"run_completed"), "{events:?}");
    let view = fleet
        .call("GET", &format!("/api/brain/runs/{id}/view"), Value::Null)
        .await;
    assert_eq!(view.status, 200, "{view:?}");
    assert_eq!(view.body["schema_version"], 3);
    assert_eq!(view.body["objective"], "inspect repository");
    assert_eq!(view.body["input_names"], json!(["repo"]));
    assert_eq!(
        view.body["rounds"][0]["operations"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(view.body["capabilities"][0].get("definition").is_none());
    let stream = fleet
        .response("GET", &format!("/api/brain/runs/{id}/events?after=0"))
        .await;
    assert_eq!(stream.status(), 200);
    let body = axum::body::to_bytes(stream.into_body(), MAX_FRAME_BYTES)
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("operation_terminal"), "{text}");
    assert!(text.contains("run_completed"), "{text}");
    assert!(
        fleet.nodes[0]
            .indexes()
            .await
            .unwrap()
            .iter()
            .any(|index| index.id == operations[0]["execution_id"]),
        "child execution index missing"
    );
    let replay = fleet
        .call("POST", "/api/brain/runs", scheduler_support::request(id))
        .await;
    assert_eq!(replay.status, 202, "{replay:?}");
    fleet.shutdown().await;
}
