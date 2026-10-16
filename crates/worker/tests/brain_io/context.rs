use super::support;
#[path = "../scheduler_v4/plan.rs"]
mod plan;
use opencoder_core::{brain::layered::LayeredSnapshot, brain::ArtifactRef, fleet::*};
use opencoder_node::fleet::NodeService;
use serde_json::{json, Value};

#[derive(Default)]
struct NewBindingModel(std::sync::Mutex<Vec<Value>>);

impl opencoder_llm::ChatStream for NewBindingModel {
    fn chat_stream(
        &self,
        request: opencoder_llm::ChatRequest,
    ) -> anyhow::Result<tokio::sync::mpsc::Receiver<opencoder_llm::LlmEvent>> {
        self.0.lock().unwrap().push(serde_json::from_str(
            &request.messages.last().unwrap().text(),
        )?);
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        sender.try_send(opencoder_llm::LlmEvent::Completed {
            text: json!({"decision":"dispatch_layer","layer":1,"reason":"attempt new wire",
                "assignments":[{"node_id":"scan","capability_id":"cap-scan","reason":"use scan",
                    "inputs":{"repo":{"kind":"execution","execution_id":"agent-terminal","source":"artifacts","path":""}}}]}).to_string(),
            tool_calls:vec![],usage:None,
        })?;
        Ok(receiver)
    }
}

#[tokio::test]
async fn legacy_root_rejects_context_upgrade_and_worker_first_cannot_publish_new_bindings() {
    let (_config, _home) = plan::isolated_config();
    let temp = tempfile::tempdir().unwrap();
    let model = std::sync::Arc::new(NewBindingModel::default());
    let node = support::worker(temp.path(), model.clone()).await;
    let id = "brain-legacy-wire";
    let reference = ExecutionRef {
        id: id.into(),
        kind: ExecutionKind::Brain,
    };
    let input = json!({"schema_version":7,"layered_request":plan::request(id),"frozen_capabilities":plan::catalog()});
    assert_eq!(
        node.handle(NodeOperation::Create {
            assignment: support::assignment(
                &node,
                id,
                ExecutionKind::Brain,
                input,
                Some(json!({}))
            )
        })
        .await
        .status,
        200
    );
    support::settled(&node, id).await;
    let before = node
        .handle(NodeOperation::Brain {
            execution: reference.clone(),
            action: "snapshot".into(),
            input: Value::Null,
        })
        .await;
    let snapshot: LayeredSnapshot = serde_json::from_value(before.body.clone()).unwrap();
    let mut context = plan::next_context(id, &snapshot);
    context.execution_io_version = 1;
    let rejected = node
        .handle(NodeOperation::Brain {
            execution: reference.clone(),
            action: "layered_context".into(),
            input: json!(context),
        })
        .await;
    assert_eq!(rejected.status, 409, "{rejected:?}");
    let unchanged = node
        .handle(NodeOperation::Brain {
            execution: reference.clone(),
            action: "snapshot".into(),
            input: Value::Null,
        })
        .await;
    assert_eq!(unchanged.body, before.body);
    context.execution_io_version = 0;
    let wire = json!(context);
    assert!(wire.get("execution_io_version").is_none());
    let accepted = node
        .handle(NodeOperation::Brain {
            execution: reference.clone(),
            action: "layered_context".into(),
            input: wire,
        })
        .await;
    assert_eq!(accepted.status, 200, "{accepted:?}");
    let detail = support::settled(&node, id).await;
    assert!(detail["annotations"]["layered_intent"].is_null());
    {
        let contexts = model.0.lock().unwrap();
        assert_eq!(contexts.len(), 3);
        assert!(contexts
            .iter()
            .all(|context| context.get("artifact_bindings").is_none()));
    }
    let blocked = node
        .handle(NodeOperation::Brain {
            execution: reference,
            action: "snapshot".into(),
            input: Value::Null,
        })
        .await;
    assert_eq!(blocked.body["run"]["phase"], "blocked");
    assert_eq!(blocked.body["operations"], json!([]));
    node.shutdown().await.unwrap();
}

#[tokio::test]
async fn worker_rejects_a_foreign_artifact_index_without_advancing_the_root() {
    let (_config, _home) = plan::isolated_config();
    let temp = tempfile::tempdir().unwrap();
    let node = support::worker(temp.path(), support::mock()).await;
    let id = "brain-io-context";
    let reference = ExecutionRef {
        id: id.into(),
        kind: ExecutionKind::Brain,
    };
    let input = json!({"schema_version":7,"execution_io_version":1,"layered_request":plan::request(id),
        "frozen_capabilities":plan::catalog()});
    assert_eq!(
        node.handle(NodeOperation::Create {
            assignment: support::assignment(
                &node,
                id,
                ExecutionKind::Brain,
                input,
                Some(json!({}))
            ),
        })
        .await
        .status,
        200
    );
    support::settled(&node, id).await;
    let before = node
        .handle(NodeOperation::Brain {
            execution: reference.clone(),
            action: "snapshot".into(),
            input: Value::Null,
        })
        .await;
    assert_eq!(before.status, 200, "{before:?}");
    let snapshot: LayeredSnapshot = serde_json::from_value(before.body.clone()).unwrap();
    let mut context = plan::next_context(id, &snapshot);
    context.execution_io_version = 1;
    context.execution_artifacts.insert(
        "agent-foreign-run".into(),
        vec![ArtifactRef {
            execution: ExecutionRef {
                id: "agent-foreign-run".into(),
                kind: ExecutionKind::Agent,
            },
            step: "workspace".into(),
            index: None,
            file: "report.txt".into(),
            bytes: 0,
            sha256: "a".repeat(64),
        }],
    );
    let rejected = node
        .handle(NodeOperation::Brain {
            execution: reference.clone(),
            action: "layered_context".into(),
            input: json!(context),
        })
        .await;
    assert_eq!(rejected.status, 409, "{rejected:?}");
    assert!(rejected
        .body
        .to_string()
        .contains("artifact index does not belong"));
    let after = node
        .handle(NodeOperation::Brain {
            execution: reference,
            action: "snapshot".into(),
            input: Value::Null,
        })
        .await;
    assert_eq!(after.status, 200);
    assert_eq!(after.body, before.body);
    node.shutdown().await.unwrap();
}
