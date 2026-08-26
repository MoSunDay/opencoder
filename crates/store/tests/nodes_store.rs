//! Functional tests for the multi-node distributed-execution store API
//! (`nodes` registry + `node_tasks` dispatch queue).
//!
//! Behavior contracts:
//! - register_list_get_delete_roundtrip: CRUD + same-name re-registration
//!   keeps the original ULID (FKs stay linked) and refreshes liveness fields
//! - delete_node_cascades_to_tasks_and_synthetic_sessions: double-FK cascade
//! - dispatch_creates_synthetic_session_with_task_type_node
//! - claim_is_fifo_single_active_and_per_node_isolated (+ concurrency:
//!   8 racing claimers never double-dispatch thanks to BEGIN IMMEDIATE + CAS)
//! - status_transition_grid_rejects_illegal_moves: state-machine enforcement,
//!   terminal freeze, finished_at stamping, node slot release
//!
//! These run against a real on-disk libsql file (tempdir) so WAL semantics
//! are exercised truthfully.

use std::sync::Arc;

use opencoder_store::{LibsqlStore, NodeTaskStatus, Store, TASK_TYPE_NODE};
use tempfile::TempDir;

const NODE_A: &str = "node-alpha";
const NODE_B: &str = "node-beta";

async fn fresh() -> (TempDir, LibsqlStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = LibsqlStore::open(dir.path().join("test.db")).await.unwrap();
    (dir, store)
}

async fn register(store: &LibsqlStore, name: &str, now_ms: i64) -> opencoder_store::NodeRecord {
    store
        .register_node(name, Some("v1"), Some("/tmp/wd"), now_ms)
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

#[tokio::test]
async fn register_list_get_delete_roundtrip() {
    let (_dir, store) = fresh().await;

    let first = register(&store, NODE_A, 1000).await;
    assert_eq!(first.name, NODE_A);
    assert_eq!(first.version.as_deref(), Some("v1"));
    assert_eq!(first.first_seen, 1000);
    assert_eq!(first.last_seen_at, 1000);
    assert_eq!(first.last_status, "online");
    assert_eq!(first.last_task_id, None);

    // Same-name re-registration: original ULID survives (dispatched tasks keep
    // their FK), timestamps refresh, new metadata lands.
    dispatch(&store, "t-reg", 1, &first.id, 1100).await;
    let again = register(&store, NODE_A, 9000).await;
    assert_eq!(again.id, first.id, "re-register must reuse the node id");
    assert_eq!(again.first_seen, 1000, "first_seen is birth metadata");
    assert_eq!(again.last_seen_at, 9000, "last_seen must refresh");
    assert_eq!(again.version.as_deref(), Some("v1"));
    // Old tasks remain reachable through the retained id.
    assert!(store.get_node_task("t-reg").await.unwrap().is_some());
    assert_eq!(
        store.get_node(&first.id).await.unwrap().unwrap().name,
        NODE_A
    );

    let listed = store.list_nodes().await.unwrap();
    assert_eq!(listed.len(), 1);

    // A distinct name is a distinct node row.
    let b = register(&store, NODE_B, 1001).await;
    assert_eq!(store.list_nodes().await.unwrap().len(), 2);

    // Deletion targets the exact row; only that node disappears.
    store.delete_node(&b.id).await.unwrap();
    assert!(store.get_node(&b.id).await.unwrap().is_none());
    assert!(store.get_node(&first.id).await.unwrap().is_some());
    assert_eq!(store.list_nodes().await.unwrap().len(), 1);
}

#[tokio::test]
async fn delete_node_cascades_to_tasks_and_synthetic_sessions() {
    let (_dir, store) = fresh().await;
    let node = register(&store, NODE_A, 1000).await;
    dispatch(&store, "t-del", 1, &node.id, 1100).await;

    store.delete_node(&node.id).await.unwrap();

    assert!(store.get_node(&node.id).await.unwrap().is_none());
    assert!(store.list_nodes().await.unwrap().is_empty());
    // node_tasks row gone...
    assert!(store.get_node_task("t-del").await.unwrap().is_none());
    // ...and the synthetic session cascaded away with it.
    assert!(store.get_session("sess-1").await.unwrap().is_none());
}

#[tokio::test]
async fn dispatch_creates_synthetic_session_with_task_type_node() {
    let (_dir, store) = fresh().await;
    let node = register(&store, NODE_A, 1000).await;
    dispatch(&store, "t-sess", 7, &node.id, 1100).await;

    let meta = store
        .get_session("sess-7")
        .await
        .unwrap()
        .expect("synthetic session exists");
    assert_eq!(meta.task_type.as_deref(), Some(TASK_TYPE_NODE));
    assert_eq!(meta.title.as_deref(), Some("task t-sess"));
    assert_eq!(meta.agent.as_deref(), Some("build"));
    assert_eq!(meta.model.as_deref(), Some("glm-5.2"));

    let task = store.get_node_task("t-sess").await.unwrap().unwrap();
    assert_eq!(task.status, NodeTaskStatus::Pending);
    assert_eq!(task.node_id, node.id);
    assert_eq!(task.session_id, "sess-7");
    assert_eq!(task.prompt, "prompt-t-sess");
    assert!(!task.cancel_requested);
    assert_eq!(task.claimed_at, None);
    assert_eq!(task.finished_at, None);
}

/// Dispatching to an unknown node must fail loudly (no orphan queue rows).
#[tokio::test]
async fn dispatch_to_unknown_node_errors() {
    let (_dir, store) = fresh().await;
    let err = store
        .dispatch_node_task("t-x", "sess-x", "no-such-node", None, "p", None, None, 1)
        .await;
    assert!(err.is_err());
}

#[tokio::test]
async fn claim_is_fifo_single_active_and_per_node_isolated() {
    let (_dir, store) = fresh().await;
    let a = register(&store, NODE_A, 1000).await;
    let b = register(&store, NODE_B, 1000).await;
    dispatch(&store, "t-a2", 1, &a.id, 1200).await; // older
    dispatch(&store, "t-a1", 2, &a.id, 1100).await; // oldest => first out

    // Empty queue yields None without error.
    let none = store.list_node_tasks(&b.id, 10).await.unwrap();
    assert!(none.is_empty());
    assert!(
        claim_nothing(&store, &b.id).await,
        "queue-less node claims nothing"
    );

    let first = store
        .claim_next_node_task(&a.id, 2000)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.id, "t-a1", "FIFO: oldest created_at wins");
    assert_eq!(first.status, NodeTaskStatus::Running);
    assert_eq!(first.claimed_at, Some(2000));

    // Single-active-task policy: node A is busy/cancelling-carrying.
    assert!(
        claim_nothing(&store, &a.id).await,
        "busy node may not claim a second task"
    );
    // ...but an independent node keeps pulling its own queue.
    dispatch(&store, "t-b1", 3, &b.id, 1300).await;
    let got_b = store
        .claim_next_node_task(&b.id, 2100)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got_b.id, "t-b1");

    // Finishing releases node A's slot; the queue continues FIFO with t-a2.
    store
        .update_node_task_status(&first.id, NodeTaskStatus::Done, None, 2200)
        .await
        .unwrap();
    let second = store
        .claim_next_node_task(&a.id, 2300)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(second.id, "t-a2");

    // Node snapshot reflects the busy handoff: still-working B keeps latest task id.
    let node_a = store.get_node(&a.id).await.unwrap().unwrap();
    assert_eq!(node_a.last_task_id.as_deref(), Some("t-a2"));
    assert_eq!(node_a.last_status, "busy");
}

async fn claim_nothing(store: &LibsqlStore, node_id: &str) -> bool {
    matches!(store.claim_next_node_task(node_id, 2050).await, Ok(None))
}

/// 8 concurrent claimers over one node's queue: every task handed out exactly
/// once, no duplicates — guaranteed by BEGIN IMMEDIATE serialization plus the
/// `UPDATE ... WHERE status='pending'` CAS.
#[tokio::test]
async fn concurrent_claims_never_double_dispatch() {
    const CLAIMERS: usize = 8;
    const TASKS: usize = 5;

    let (_dir, store) = fresh().await;
    let store = Arc::new(store);
    let node = register(&store, NODE_A, 1000).await;
    for i in 0..TASKS {
        dispatch(
            &store,
            &format!("t-c{i}"),
            100 + i,
            &node.id,
            1100 + i as i64,
        )
        .await;
    }

    let mut handles = Vec::new();
    for _ in 0..CLAIMERS {
        let st = Arc::clone(&store);
        let node_id = node.id.clone();
        handles.push(tokio::spawn(async move {
            let mut claimed = Vec::new();
            // Each "worker" processes what it wins, frees the slot, repeats
            // until the queue reports empty.
            while let Some(task) = st.claim_next_node_task(&node_id, 2000).await.unwrap() {
                claimed.push(task.id.clone());
                st.update_node_task_status(&task.id, NodeTaskStatus::Done, None, 2100)
                    .await
                    .unwrap();
            }
            claimed
        }));
    }

    let mut all_claimed = Vec::new();
    for h in handles {
        all_claimed.extend(h.await.unwrap());
    }

    assert_eq!(
        all_claimed.len(),
        TASKS,
        "exactly one claim per queued task across all racers"
    );
    all_claimed.sort();
    all_claimed.dedup();
    assert_eq!(
        all_claimed.len(),
        TASKS,
        "claimed task ids must be unique (CAS prevented double-dispatch)"
    );

    let node_after = store.get_node(&node.id).await.unwrap().unwrap();
    assert_eq!(
        node_after.last_status, "idle",
        "drained queue leaves node idle"
    );
    assert!(store
        .list_node_tasks(&node.id, 50)
        .await
        .unwrap()
        .iter()
        .all(|t| { t.status == NodeTaskStatus::Done && t.finished_at.is_some() }));
}

#[tokio::test]
async fn status_transition_grid_rejects_illegal_moves() {
    let (_dir, store) = fresh().await;
    let node = register(&store, NODE_A, 1000).await;

    // Legal ladder: pending -> running -> done (stamp finished_at + idle node).
    dispatch(&store, "t-ok", 1, &node.id, 1100).await;
    store
        .update_node_task_status("t-ok", NodeTaskStatus::Running, None, 1200)
        .await
        .unwrap();
    store
        .update_node_task_status("t-ok", NodeTaskStatus::Done, None, 1300)
        .await
        .unwrap();
    let done = store.get_node_task("t-ok").await.unwrap().unwrap();
    assert_eq!(done.status, NodeTaskStatus::Done);
    assert_eq!(done.finished_at, Some(1300));
    assert_eq!(done.error, None);

    // Illegal skip: pending -> done (the task never ran).
    dispatch(&store, "t-skip", 2, &node.id, 1150).await;
    let err = store
        .update_node_task_status("t-skip", NodeTaskStatus::Done, None, 1400)
        .await;
    assert!(err.is_err(), "pending -> done must be rejected");

    // Terminal freeze: done accepts nothing further.
    let err = store
        .update_node_task_status("t-ok", NodeTaskStatus::Error, Some("late"), 1500)
        .await;
    assert!(err.is_err(), "terminal tasks are frozen");
    let frozen = store.get_node_task("t-ok").await.unwrap().unwrap();
    assert_eq!(frozen.status, NodeTaskStatus::Done);
    assert_eq!(frozen.error, None, "rejected write left no residue");

    // Unknown task errors rather than silently succeeding.
    assert!(store
        .update_node_task_status("ghost", NodeTaskStatus::Done, None, 1600)
        .await
        .is_err());

    // Cancelling ladder is also legal: pending -> cancelling -> error. The
    // task was never claimed, so the node's slot pointer is untouched.
    dispatch(&store, "t-can", 3, &node.id, 1170).await;
    store
        .update_node_task_status("t-can", NodeTaskStatus::Cancelling, None, 1610)
        .await
        .unwrap();
    store
        .update_node_task_status("t-can", NodeTaskStatus::Error, Some("aborted"), 1620)
        .await
        .unwrap();
    let aborted = store.get_node_task("t-can").await.unwrap().unwrap();
    assert_eq!(aborted.error.as_deref(), Some("aborted"));
    assert_eq!(aborted.finished_at, Some(1620));

    // Now claim the remaining t-skip and finish it: because it is the node's
    // current last task, finishing releases the busy slot (`idle`) while the
    // pointer stays so UIs can keep showing recent work.
    let claimed = store
        .claim_next_node_task(&node.id, 1630)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed.id, "t-skip");
    let node_busy = store.get_node(&node.id).await.unwrap().unwrap();
    assert_eq!(node_busy.last_status, "busy");
    store
        .update_node_task_status("t-skip", NodeTaskStatus::Done, None, 1640)
        .await
        .unwrap();
    let node_now = store.get_node(&node.id).await.unwrap().unwrap();
    assert_eq!(
        node_now.last_status, "idle",
        "finished last task releases the slot"
    );
    assert_eq!(
        node_now.last_task_id.as_deref(),
        Some("t-skip"),
        "last_task_id is history, not cleared"
    );
}
