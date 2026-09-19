//! V3 root execution keeps a recoverable wake marker and never embeds child
//! execution output in the root journal.
mod support;

use opencoder_core::fleet::*;
use opencoder_node::fleet::NodeService;
use serde_json::json;
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
