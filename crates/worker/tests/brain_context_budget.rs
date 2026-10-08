#![cfg(not(windows))]
mod support;
use opencoder_core::fleet::*;
use opencoder_node::fleet::NodeService;
use serde_json::{json, Value};
use support::*;

fn input(width: usize) -> Value {
    json!({"schema_version":7,"frozen_capabilities":[{"capability_id":"agent","kind":"agent","target":"act","input_desc":"task","output_desc":"evidence","definition":{},"version":"1"}],
        "layered_request":{"schema_version":7,"plan":{"schema_version":7,"title":"capacity","objective":"verify budget","layers":[{"layer_id":"work","title":"work","task":"work","objective":"verify","success_criteria":"evidence retained"}],
        "nodes":(0..width).map(|n| json!({"node_id":format!("n{n}"),"layer_id":"work","title":"work","objective":"work","capability_id":"agent"})).collect::<Vec<_>>(),"edges":[]}}})
}

#[tokio::test]
async fn probe_and_create_reject_a_plan_before_execution_admission() {
    let (_config, _home) = isolated_config();
    let dir = tempfile::tempdir().unwrap();
    let node = worker(dir.path(), mock()).await;
    let id = "brain-over-capacity";
    let reply = node
        .handle(NodeOperation::Brain {
            execution: ExecutionRef {
                id: id.into(),
                kind: ExecutionKind::Brain,
            },
            action: "capability_probe".into(),
            input: input(32),
        })
        .await;
    assert_eq!(reply.status, 413, "{reply:?}");
    let reply = node
        .handle(NodeOperation::Create {
            assignment: assignment(&node, id, ExecutionKind::Brain, input(32), Some(json!({}))),
        })
        .await;
    assert_eq!(reply.status, 413, "{reply:?}");
    assert!(node.indexes().await.unwrap().is_empty());
    node.shutdown().await.unwrap();
}

#[tokio::test]
async fn rejected_human_input_does_not_append_events_or_advance_generation() {
    let (_config, _home) = isolated_config();
    let dir = tempfile::tempdir().unwrap();
    let node = worker(dir.path(), mock()).await;
    let id = "brain-guidance-capacity";
    let reply = node
        .handle(NodeOperation::Create {
            assignment: assignment(&node, id, ExecutionKind::Brain, input(1), Some(json!({}))),
        })
        .await;
    assert_eq!(reply.status, 200, "{reply:?}");
    settled(&node, id).await;
    let reference = ExecutionRef {
        id: id.into(),
        kind: ExecutionKind::Brain,
    };
    let mut rejected = false;
    for _ in 0..30 {
        let before = node
            .handle(NodeOperation::Brain {
                execution: reference.clone(),
                action: "snapshot".into(),
                input: Value::Null,
            })
            .await;
        let reply = node
            .handle(NodeOperation::Brain {
                execution: reference.clone(),
                action: "human_input".into(),
                input: json!({"text":"中".repeat(1000)}),
            })
            .await;
        if reply.status == 413 {
            let after = node
                .handle(NodeOperation::Brain {
                    execution: reference.clone(),
                    action: "snapshot".into(),
                    input: Value::Null,
                })
                .await;
            assert_eq!(before.body, after.body);
            rejected = true;
            break;
        }
        assert_eq!(reply.status, 200, "{reply:?}");
    }
    assert!(
        rejected,
        "cumulative guidance must have a finite admission budget"
    );
    node.shutdown().await.unwrap();
}
