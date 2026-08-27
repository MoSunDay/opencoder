//! Cancel-delivery and liveness contracts for the node-task queue.
//!
//! - request_node_task_cancel is idempotent, only fires on pending/running,
//!   and reports the pre-cancel status so callers can distinguish
//!   queued-cancels from in-flight ones
//! - heartbeat_node delivers exactly the cancelling tasks as commands,
//!   refreshes last_seen_at, collapses non-busy status toward idle without
//!   demoting a working (`busy`) node, and hard-errors on unknown ids

use opencoder_store::{LibsqlStore, NodeTaskStatus, SessionMeta, Store};
use tempfile::TempDir;

const NODE_A: &str = "node-alpha";

async fn fresh() -> (TempDir, LibsqlStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = LibsqlStore::open(dir.path().join("test.db")).await.unwrap();
    (dir, store)
}

async fn register(store: &LibsqlStore, name: &str, now_ms: i64) -> opencoder_store::NodeRecord {
    store
        .register_node(name, Some("v1"), Some("/tmp/wd"), None, now_ms)
        .await
        .unwrap()
}

async fn dispatch(
    store: &LibsqlStore,
    task_id: &str,
    session_seq: usize,
    node_id: &str,
    created_at: i64,
) {
    store
        .dispatch_node_task(
            task_id,
            &format!("sess-{session_seq}"),
            node_id,
            Some(format!("task {task_id}").as_str()),
            format!("prompt-{task_id}").as_str(),
            Some("build"),
            Some("glm-5.2"),
            created_at,
        )
        .await
        .unwrap();
}

fn sessmeta(id: &str, now: i64) -> SessionMeta {
    SessionMeta {
        id: id.to_string(),
        title: Some(format!("title-{id}")),
        agent: Some("act".into()),
        model: Some("glm-5.2".into()),
        autopilot_mode: None,
        workdir_hash: Some("h".into()),
        created_at: now,
        updated_at: now,
        summary: None,
        summary_seq: None,
        summary_images: vec![],
        handoff_seq: None,
        handoff_plan: None,
        skill: None,
        task_type: None,
        requirement: None,
        plan_snapshot: None,
        plan_input_count: 0,
    }
}

#[tokio::test]
async fn request_cancel_is_idempotent_and_heartbeat_delivers_it() {
    let (_dir, store) = fresh().await;
    let node = register(&store, NODE_A, 1000).await;

    // Cancelling an inactive/unknown task is a harmless no-op returning None.
    dispatch(&store, "t-idle", 1, &node.id, 1100).await;
    assert_eq!(store.request_node_task_cancel("ghost").await.unwrap(), None);
    // Heartbeat with nothing cancelling returns an empty command list.
    assert!(store
        .heartbeat_node(&node.id, 1110)
        .await
        .unwrap()
        .is_empty());

    // Cancel while pending: the caller learns it was still queued (can be
    // turned straight into cancelled by the dispatcher if it wants).
    let prev = store.request_node_task_cancel("t-idle").await.unwrap();
    assert_eq!(prev, Some(NodeTaskStatus::Pending));

    // Idempotency: a second cancel finds status=cancelling, not pending/running.
    assert_eq!(
        store.request_node_task_cancel("t-idle").await.unwrap(),
        None
    );

    // The node's next heartbeat delivers exactly this cancel instruction.
    assert_eq!(
        store.heartbeat_node(&node.id, 1120).await.unwrap(),
        vec!["t-idle".to_string()]
    );

    // Collapse to done; the cancelled command disappears from the poll.
    store
        .update_node_task_status("t-idle", NodeTaskStatus::Done, None, 1130)
        .await
        .unwrap();
    assert!(store
        .heartbeat_node(&node.id, 1140)
        .await
        .unwrap()
        .is_empty());

    // Cancel after completion is also a no-op (terminal).
    assert_eq!(
        store.request_node_task_cancel("t-idle").await.unwrap(),
        None
    );

    // Running flavor: callers can distinguish queued-vs-in-flight cancels.
    dispatch(&store, "t-run", 2, &node.id, 1200).await;
    store
        .claim_next_node_task(&node.id, 1210)
        .await
        .unwrap()
        .unwrap();
    let prev = store.request_node_task_cancel("t-run").await.unwrap();
    assert_eq!(prev, Some(NodeTaskStatus::Running));
    assert_eq!(
        store.heartbeat_node(&node.id, 1220).await.unwrap(),
        vec!["t-run".to_string()]
    );
}

/// heartbeat_node refreshes liveness; busy nodes keep their status while idle
/// ones stay idle-only-by-heartbeat (`online` collapses to `idle`).

#[tokio::test]
async fn heartbeat_touches_liveness_but_respects_busy() {
    let (_dir, store) = fresh().await;
    // Also proves a freshly created session isn't disturbed by the new tables.
    store.create_session(&sessmeta("plain", 999)).await.unwrap();

    let node = register(&store, NODE_A, 1000).await;
    assert_eq!(
        store.heartbeat_node(&node.id, 5000).await.unwrap(),
        Vec::<String>::new()
    );
    let touched = store.get_node(&node.id).await.unwrap().unwrap();
    assert_eq!(touched.last_seen_at, 5000);
    assert_eq!(
        touched.last_status, "idle",
        "non-busy collapses toward idle"
    );

    dispatch(&store, "t-busy", 3, &node.id, 5100).await;
    store
        .claim_next_node_task(&node.id, 5200)
        .await
        .unwrap()
        .unwrap();
    store.heartbeat_node(&node.id, 5300).await.unwrap();
    let busy = store.get_node(&node.id).await.unwrap().unwrap();
    assert_eq!(
        busy.last_status, "busy",
        "heartbeat must not demote a working node"
    );

    // An unknown node id hard-errors so infrastructure mistakes surface.
    assert!(store.heartbeat_node("missing", 5400).await.is_err());

    assert_eq!(
        store
            .get_session("plain")
            .await
            .unwrap()
            .unwrap()
            .created_at,
        999
    );
}
