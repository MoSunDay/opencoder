#![cfg(not(windows))]
mod support;
use opencoder_core::fleet::*;
use opencoder_node::fleet::NodeService;
use serde_json::json;
use support::*;

#[tokio::test]
async fn dag_terminal_failure_is_retained_in_execution_detail_and_after_restart() {
    let _host_config = isolated_config();
    let directory = tempfile::tempdir().unwrap();
    let client = mock();
    let (node, _native, _bridge) = dag_worker(directory.path(), client.clone()).await;
    stage_binary(
        &directory.path().join("node"),
        "fails",
        "int main(void) { return 7; }",
    );
    let definition = json!({"name":"fails","steps":[{"name":"execute",
        "kind":{"type":"binary","resource":"fails"}}]});
    let reply = node
        .handle(NodeOperation::Create {
            assignment: assignment(
                &node,
                "dag-failure",
                ExecutionKind::Dag,
                json!({}),
                Some(definition),
            ),
        })
        .await;
    assert_eq!(reply.status, 200, "{reply:?}");
    let detail = settled(&node, "dag-failure").await;
    assert_eq!(detail["execution"]["status"], "error", "{detail}");
    assert_eq!(detail["result"]["status"], "error");
    assert_eq!(detail["result"]["run_id"], "dag-failure");
    assert!(std::path::Path::new(detail["result"]["artifact_root"].as_str().unwrap()).is_dir());
    let error = detail["error"]
        .as_str()
        .expect("DAG must retain the terminal cause");
    assert!(error.contains("execute"), "{error}");
    assert!(error.contains('7'), "{error}");
    node.shutdown().await.unwrap();
    drop(node);
    let node = worker(directory.path(), client).await;
    let retained = settled(&node, "dag-failure").await;
    assert_eq!(retained["error"], detail["error"]);
    assert_eq!(retained["execution"]["status"], "error");
    node.shutdown().await.unwrap();
}
