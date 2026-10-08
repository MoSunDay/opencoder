#![cfg(not(windows))]
//! A finite Brain decision consumes frozen control evidence, not Agent files.
#[path = "scheduler_v4/client.rs"]
mod client;
#[path = "scheduler_v4/control.rs"]
mod control;
#[path = "scheduler_v4/plan.rs"]
mod plan;
mod support;

use client::LayeredClient;
use opencoder_core::{brain::layered::LayeredPhase, fleet::*};
use opencoder_node::fleet::NodeService;
use serde_json::json;
use std::sync::Arc;
use support::*;

#[tokio::test]
async fn finite_root_decides_without_agent_files_but_agent_children_still_require_them() {
    let (_scope, home) = isolated_brain_config();
    let source = home.path().join("unavailable-agent-export");
    std::fs::write(
        opencoder_core::Config::global_config_path().unwrap(),
        json!({"context_limit":1000000,"agent":{"agents_dir":source}}).to_string(),
    )
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let client = Arc::new(LayeredClient::new());
    let node = worker(directory.path(), client.clone()).await;
    let id = "brain-finite-resources";
    let request = json!({"schema_version":7,"layered_request":plan::request(id),
        "frozen_capabilities":plan::catalog()});
    let original = assignment(&node, id, ExecutionKind::Brain, request, Some(json!({})));
    let reply = node
        .handle(NodeOperation::Create {
            assignment: original.clone(),
        })
        .await;
    assert_eq!(reply.status, 200, "{reply:?}");
    settled(&node, id).await;

    let root = directory.path().join("node/brain").join(id);
    assert_eq!(
        std::fs::read_dir(root.join("resources")).unwrap().count(),
        0
    );
    assert!(
        !source.exists(),
        "Root must not create or read the absent pool"
    );
    control::decide_next_layer(&node, id).await;
    let waiting = control::wait_phase(&node, id, LayeredPhase::Waiting).await;
    assert_eq!(waiting.operations.len(), 1);
    assert_eq!(client.decisions(), 1);
    assert!(client
        .requests
        .lock()
        .unwrap()
        .iter()
        .all(|request| request.tools.is_empty()));
    settled(&node, id).await;
    let journal = root.join("execution.json");
    let frozen = std::fs::read(&journal).unwrap();
    let replay = node
        .handle(NodeOperation::Create {
            assignment: original,
        })
        .await;
    assert_eq!(replay.status, 200, "{replay:?}");
    for field in ["id", "kind", "node_id", "created_at"] {
        assert_eq!(
            replay.body[field], reply.body[field],
            "replay changed {field}"
        );
    }
    assert_eq!(
        std::fs::read(journal).unwrap(),
        frozen,
        "replay changed frozen resources or intent"
    );
    assert_eq!(client.decisions(), 1, "replay cannot decide a second time");

    // Agent execution remains a separate admission with its own frozen pool.
    let leaf = node
        .handle(NodeOperation::Create {
            assignment: assignment(
                &node,
                "agent-missing-pool",
                ExecutionKind::Agent,
                json!({"prompt":"bounded child task"}),
                None,
            ),
        })
        .await;
    assert_eq!(leaf.status, 400, "{leaf:?}");
    assert!(
        leaf.body.to_string().contains("resource mount unavailable"),
        "{leaf:?}"
    );
    assert!(!directory
        .path()
        .join("node/agent/agent-missing-pool/resources")
        .exists());
    node.shutdown().await.unwrap();
}
