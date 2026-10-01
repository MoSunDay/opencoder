//! Invalid resource files must fail before durable execution admission.
#![cfg(unix)]
#[path = "../support/mod.rs"]
mod support;

use opencoder_core::fleet::*;
use opencoder_node::fleet::NodeService;
use serde_json::json;
use std::time::Duration;

#[tokio::test]
async fn fifo_resource_is_rejected_without_waiting_for_a_writer_or_accepting_execution() {
    let (_guard, home) = support::isolated_config();
    let directory = tempfile::tempdir().unwrap();
    let worker = support::worker(directory.path(), support::mock()).await;
    let card = home.path().join(".opencoder/agents/blocked-card");
    std::fs::create_dir_all(&card).unwrap();
    let fifo = card.join("meta.json");
    assert!(std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap()
        .success());
    let assignment = support::assignment(
        &worker,
        "agent-slow-resources",
        ExecutionKind::Agent,
        json!({"prompt":""}),
        None,
    );
    let reply = tokio::time::timeout(
        Duration::from_secs(5),
        worker.handle(NodeOperation::Create { assignment }),
    )
    .await
    .expect("resource preflight waited for a FIFO writer");
    assert_eq!(
        reply.status, 400,
        "caller-scoped invalid pool must be rejected: {reply:?}"
    );
    assert!(reply.body.to_string().contains("regular file"), "{reply:?}");
    assert!(
        worker.indexes().await.unwrap().is_empty(),
        "failed preflight must not accept execution"
    );
    worker.shutdown().await.unwrap();
}
