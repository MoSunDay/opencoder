//! Lost-node convergence contracts (`converge_lost_node_tasks`).
//!
//! Nodes whose latest heartbeat is older than `stale_ms` are presumed dead;
//! their `running`/`cancelling` tasks are zombie rows that would block the
//! single-active claim guard forever. The sweep collapses them to
//! `error("node lost")` with `finished_at` stamped and the node slot
//! released, while leaving fresh nodes, terminal rows and `pending` rows
//! untouched. All timestamps are passed explicitly, so no sleeping.

use opencoder_store::{LibsqlStore, NodeRecord, NodeTaskStatus, Store};
use tempfile::TempDir;

const STALE_MS: i64 = 400;

async fn fresh() -> (TempDir, LibsqlStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = LibsqlStore::open(dir.path().join("test.db")).await.unwrap();
    (dir, store)
}

async fn register(store: &LibsqlStore, name: &str, now_ms: i64) -> NodeRecord {
    store
        .register_node(name, Some("v1"), Some("/tmp/wd"), None, now_ms)
        .await
        .unwrap()
}

#[allow(clippy::too_many_arguments)]
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

/// Run a task through pending -> running (claimed, node marked busy).
async fn run_to_running(store: &LibsqlStore, task_id: &str, node_id: &str, t: i64) {
    dispatch(store, task_id, t as usize, node_id, t).await;
    store.claim_next_node_task(node_id, t + 1).await.unwrap();
}

/// Stale node holding one running and one cancelling task collapses both.
#[tokio::test]
async fn stale_node_running_and_cancelling_both_collapse() {
    let (_dir, store) = fresh().await;
    let node = register(&store, "node-lost", 1_000).await;

    // last_seen_at == 1000; at now=1401 the gap is 401 > 400 -> lost.
    run_to_running(&store, "t-run", &node.id, 1_005).await;
    dispatch(&store, "t-can", 1_006, &node.id, 1_006).await;
    store
        .request_node_task_cancel("t-can")
        .await
        .unwrap()
        .unwrap();

    let converged = store
        .converge_lost_node_tasks(1_401, STALE_MS)
        .await
        .unwrap();
    assert_eq!(converged.len(), 2, "both active tasks converge");
    let mut ids: Vec<&str> = converged.iter().map(|t| t.id.as_str()).collect();
    ids.sort_unstable();
    assert_eq!(ids, ["t-can", "t-run"]);
    for record in &converged {
        assert_eq!(record.status, NodeTaskStatus::Error);
        assert_eq!(record.error.as_deref(), Some("node lost"));
        assert_eq!(record.finished_at, Some(1_401), "terminal stamp applied");
    }

    // Busy slot released exactly like a terminal update_status write.
    let released = store.get_node(&node.id).await.unwrap().unwrap();
    assert_eq!(released.last_status, "idle");
}

/// A heartbeating (strictly fresher than stale_ms) node keeps its running
/// task untouched; nothing is returned for it.
#[tokio::test]
async fn fresh_node_running_is_untouched() {
    let (_dir, store) = fresh().await;
    // last_seen_at = now - stale + 1 -> gap 399 < 400, still alive.
    let node = register(&store, "node-alive", 1_500 - STALE_MS + 1).await;
    run_to_running(&store, "t-live", &node.id, 1_505).await;

    let converged = store
        .converge_lost_node_tasks(1_500, STALE_MS)
        .await
        .unwrap();
    assert!(converged.is_empty());

    let kept = store.get_node_task("t-live").await.unwrap().unwrap();
    assert_eq!(kept.status, NodeTaskStatus::Running);
    assert_eq!(kept.error, None);
    assert_eq!(kept.finished_at, None);
}

/// Terminal rows on an equally-stale node are frozen (never re-collapsed),
/// and a second sweep over already-converged data is a no-op.
#[tokio::test]
async fn terminal_rows_frozen_and_second_sweep_is_empty() {
    let (_dir, store) = fresh().await;
    let node = register(&store, "node-mixed", 1_000).await;

    dispatch(&store, "t-done", 1, &node.id, 1_001).await;
    store.claim_next_node_task(&node.id, 1_002).await.unwrap();
    store
        .update_node_task_status("t-done", NodeTaskStatus::Done, None, 1_003)
        .await
        .unwrap();

    dispatch(&store, "t-cancelled", 2, &node.id, 1_004).await;
    store.claim_next_node_task(&node.id, 1_005).await.unwrap();
    store.request_node_task_cancel("t-cancelled").await.unwrap();
    store
        .update_node_task_status("t-cancelled", NodeTaskStatus::Cancelled, None, 1_007)
        .await
        .unwrap();

    dispatch(&store, "t-err", 3, &node.id, 1_008).await;
    store.claim_next_node_task(&node.id, 1_009).await.unwrap();
    store
        .update_node_task_status("t-err", NodeTaskStatus::Error, Some("boom"), 1_010)
        .await
        .unwrap();

    // The lone zombie in the batch.
    run_to_running(&store, "t-zombie", &node.id, 1_011).await;

    let first = store
        .converge_lost_node_tasks(1_500, STALE_MS)
        .await
        .unwrap();
    assert_eq!(first.len(), 1, "only the zombie converges");
    assert_eq!(first[0].id, "t-zombie");

    // Terminal states keep their original errors and stamps verbatim.
    let done = store.get_node_task("t-done").await.unwrap().unwrap();
    assert_eq!(done.status, NodeTaskStatus::Done);
    assert_eq!(done.finished_at, Some(1_003));
    let cancelled = store.get_node_task("t-cancelled").await.unwrap().unwrap();
    assert_eq!(cancelled.status, NodeTaskStatus::Cancelled);
    assert_eq!(cancelled.finished_at, Some(1_007));
    let err = store.get_node_task("t-err").await.unwrap().unwrap();
    assert_eq!(err.status, NodeTaskStatus::Error);
    assert_eq!(err.error.as_deref(), Some("boom"), "original error kept");
    assert_eq!(err.finished_at, Some(1_010));

    // Idempotent: everything is terminal-frozen now, nothing left to do.
    let second = store
        .converge_lost_node_tasks(1_600, STALE_MS)
        .await
        .unwrap();
    assert!(second.is_empty());
}

/// Exclusive boundary: exactly `stale_ms` old is still fresh (no collapse),
/// and `pending` tasks of genuinely lost nodes stay queued.
#[tokio::test]
async fn boundary_equality_and_pending_survive() {
    let (_dir, store) = fresh().await;

    // last_seen_at == now - stale exactly: gap 400 == 400, NOT > -> fresh.
    let edge = register(&store, "node-edge", 2_000 - STALE_MS).await;
    run_to_running(&store, "t-edge", &edge.id, 1_995).await;

    // Genuinely lost node whose only task never got claimed.
    let gone = register(&store, "node-gone", 900).await;
    dispatch(&store, "t-pend", 4, &gone.id, 905).await;

    let converged = store
        .converge_lost_node_tasks(2_000, STALE_MS)
        .await
        .unwrap();
    assert!(converged.is_empty(), "boundary node and pending survive");

    let edge_task = store.get_node_task("t-edge").await.unwrap().unwrap();
    assert_eq!(edge_task.status, NodeTaskStatus::Running);
    assert_eq!(edge_task.finished_at, None);

    let pend = store.get_node_task("t-pend").await.unwrap().unwrap();
    assert_eq!(pend.status, NodeTaskStatus::Pending);
    assert_eq!(pend.error, None);
}
