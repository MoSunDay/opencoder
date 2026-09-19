//! V3 root execution keeps a recoverable wake marker and never embeds child
//! execution output in the root journal.
mod support;

use opencoder_core::{brain::BrainSchedulerRequest, fleet::*};
use opencoder_llm::{LlmEvent, MockChatClient};
use opencoder_node::fleet::NodeService;
use serde_json::json;
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
