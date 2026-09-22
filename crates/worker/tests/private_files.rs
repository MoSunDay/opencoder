mod support;
use opencoder_core::fleet::*;
use opencoder_node::fleet::NodeService;
use serde_json::json;
use std::collections::BTreeMap;
use support::*;

#[tokio::test]
async fn private_dag_files_are_pinned_durable_and_absent_from_public_readback() {
    let _config = isolated_config();
    let temp = tempfile::tempdir().unwrap();
    let client = mock();
    let node = worker(temp.path(), client.clone()).await;
    let probe = node
        .handle(NodeOperation::Brain {
            execution: ExecutionRef {
                id: "probe".into(),
                kind: ExecutionKind::Dag,
            },
            action: "capability_probe".into(),
            input: json!({}),
        })
        .await;
    assert_eq!(probe.status, 200);
    assert!(probe.body.get("image_digest").is_none());
    let pinned = node
        .handle(NodeOperation::Brain {
            execution: ExecutionRef {
                id: "private-probe".into(),
                kind: ExecutionKind::Dag,
            },
            action: "capability_probe".into(),
            input: json!({"private_files":true}),
        })
        .await;
    assert_eq!(pinned.status, 200);
    let definition = json!({"name":"private-check","steps":[{"name":"review","kind":{"type":"agent","prompt":"inspect private file paths"}}]});
    let mut task = assignment(
        &node,
        "dag-private",
        ExecutionKind::Dag,
        json!({}),
        Some(definition.clone()),
    );
    task.private_context = Some(PrivateExecutionContext {
        expires_at_ms: opencoder_core::message::now_ms() + 60_000,
        image_digest: pinned.body["image_digest"].as_str().unwrap().into(),
        definition_sha256: opencoder_core::token_hash(&definition.to_string()),
        files: BTreeMap::from([("credential".into(), "fixture-do-not-publish".into())]),
    });
    let mut wrong = task.clone();
    wrong.private_context.as_mut().unwrap().definition_sha256 = "0".repeat(64);
    assert_eq!(
        node.handle(NodeOperation::Create { assignment: wrong })
            .await
            .status,
        409
    );
    let mut wrong = task.clone();
    wrong.private_context.as_mut().unwrap().image_digest = format!("sha256:{}", "0".repeat(64));
    assert_eq!(
        node.handle(NodeOperation::Create { assignment: wrong })
            .await
            .status,
        409
    );
    let reply = node
        .handle(NodeOperation::Create {
            assignment: task.clone(),
        })
        .await;
    assert_eq!(reply.status, 200, "{reply:?}");
    let public = settled(&node, "dag-private").await;
    assert_eq!(public["execution"]["status"], "done", "{public}");
    assert!(!public.to_string().contains("fixture-do-not-publish"));
    let path = temp
        .path()
        .join("node/private-executions/dag-private/credential");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "fixture-do-not-publish"
    );
    let requests = serde_json::to_string(
        &client
            .requests()
            .iter()
            .map(|r| &r.messages)
            .collect::<Vec<_>>(),
    )
    .unwrap();
    assert!(requests.contains("private-executions/dag-private"));
    assert!(!requests.contains("fixture-do-not-publish"));
    assert_eq!(
        node.handle(NodeOperation::Create {
            assignment: task.clone()
        })
        .await
        .status,
        200
    );
    node.shutdown().await.unwrap();
    drop(node);
    let node = worker(temp.path(), client).await;
    let public = settled(&node, "dag-private").await;
    assert!(!public.to_string().contains("fixture-do-not-publish"));
    task.private_context
        .as_mut()
        .unwrap()
        .files
        .insert("credential".into(), "changed".into());
    assert_eq!(
        node.handle(NodeOperation::Create { assignment: task })
            .await
            .status,
        409
    );
    node.shutdown().await.unwrap();
}
