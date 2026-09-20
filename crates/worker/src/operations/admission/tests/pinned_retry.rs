use super::{assignment, open};
use crate::operations::create::create;
use opencoder_core::fleet::ExecutionKind;
use serde_json::{json, Value};
use std::time::Duration;

#[tokio::test]
async fn prepared_snapshot_retry_bypasses_occupied_copy_slots() {
    let root = tempfile::tempdir().unwrap();
    let _home = opencoder_core::config::scoped_config_home(root.path().join("home"));
    let worker = open(root.path()).await;
    let original = assignment(&worker, "agent-pinned-retry", ExecutionKind::Agent);
    super::super::preparation::begin(&worker, original.clone())
        .unwrap()
        .unwrap();
    let execution = root.path().join("node/agent/agent-pinned-retry");
    crate::resources::pin(None, &execution.join("resources")).unwrap();
    let busy = worker
        .inner
        .resource_preparations
        .acquire_many(4)
        .await
        .unwrap();

    let mut conflict = original.clone();
    conflict.request.input = json!({"prompt":"changed"});
    assert_eq!(create(&worker, conflict).await.unwrap().status, 409);
    let reply = tokio::time::timeout(Duration::from_secs(10), create(&worker, original.clone()))
        .await
        .expect("frozen snapshot recovery waited for unrelated cold copies")
        .unwrap();
    assert_eq!(reply.status, 200, "{reply:?}");
    let stored: Value =
        serde_json::from_slice(&std::fs::read(execution.join("execution.json")).unwrap()).unwrap();
    assert_eq!(stored["assignment"]["request"], json!(original.request));
    assert!(!execution.join("pending-create.json").exists());
    drop(busy);
    worker.shutdown().await.unwrap();
}
