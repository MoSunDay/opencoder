//! Lost-node convergence contracts (`converge_lost_node_tasks`). Ported
//! from `tests/nodes_lost_converge.rs`: heartbeat-stale nodes have their
//! running/cancelling zombie rows collapsed to `error("node lost")`, fresh
//! nodes / pending rows / terminal rows stay put, and sweeps are
//! idempotent. All timestamps are explicit — no sleeping.

use super::*;

const STALE_MS: i64 = 400;

/// Run a task through pending -> running (claimed, node marked busy).
async fn run_to_running(store: &RdbStore, task_id: &str, node_id: &str, t: i64) {
    dispatch(store, task_id, t as usize, node_id, t).await;
    store.claim_next_node_task(node_id, t + 1).await.unwrap();
}

/// Stale node holding one running and one cancelling task collapses both.
#[tokio::test]
async fn stale_node_running_and_cancelling_both_collapse() {
    let store = mem().await;
    // last_seen_at == 1000; at now=1401 the gap is 401 > 400 -> lost.
    let node = register(&store, "node-lost", 1000).await;
    run_to_running(&store, "t-run", &node.id, 1005).await;
    dispatch(&store, "t-can", 1006, &node.id, 1006).await;
    store
        .request_node_task_cancel("t-can")
        .await
        .unwrap()
        .unwrap();

    let converged = store
        .converge_lost_node_tasks(1401, STALE_MS)
        .await
        .unwrap();
    assert_eq!(converged.len(), 2, "both active tasks converge");
    let mut ids: Vec<&str> = converged.iter().map(|t| t.id.as_str()).collect();
    ids.sort_unstable();
    assert_eq!(ids, ["t-can", "t-run"]);
    for record in &converged {
        assert_eq!(record.status, NodeTaskStatus::Error);
        assert_eq!(record.error.as_deref(), Some("node lost"));
        assert_eq!(record.finished_at, Some(1401), "terminal stamp applied");
    }

    // Busy slot released exactly like a terminal update_status write.
    let released = store.get_node(&node.id).await.unwrap().unwrap();
    assert_eq!(released.last_status, "idle");
}

/// A heartbeating (strictly fresher than stale_ms) node keeps its running
/// task untouched; nothing is returned for it.
#[tokio::test]
async fn fresh_node_running_is_untouched() {
    let store = mem().await;
    // last_seen_at = now - stale + 1 -> gap 399 < 400, still alive.
    let node = register(&store, "node-alive", 1500 - STALE_MS + 1).await;
    run_to_running(&store, "t-live", &node.id, 1505).await;

    let converged = store
        .converge_lost_node_tasks(1500, STALE_MS)
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
    let store = mem().await;
    let node = register(&store, "node-mixed", 1000).await;

    dispatch(&store, "t-done", 1, &node.id, 1001).await;
    store.claim_next_node_task(&node.id, 1002).await.unwrap();
    store
        .update_node_task_status("t-done", NodeTaskStatus::Done, None, 1003)
        .await
        .unwrap();

    dispatch(&store, "t-cancelled", 2, &node.id, 1004).await;
    store.claim_next_node_task(&node.id, 1005).await.unwrap();
    store.request_node_task_cancel("t-cancelled").await.unwrap();
    store
        .update_node_task_status("t-cancelled", NodeTaskStatus::Cancelled, None, 1007)
        .await
        .unwrap();

    dispatch(&store, "t-err", 3, &node.id, 1008).await;
    store.claim_next_node_task(&node.id, 1009).await.unwrap();
    store
        .update_node_task_status("t-err", NodeTaskStatus::Error, Some("boom"), 1010)
        .await
        .unwrap();

    // The lone zombie in the batch.
    run_to_running(&store, "t-zombie", &node.id, 1011).await;

    let first = store
        .converge_lost_node_tasks(1500, STALE_MS)
        .await
        .unwrap();
    assert_eq!(first.len(), 1, "only the zombie converges");
    assert_eq!(first[0].id, "t-zombie");

    // Terminal states keep their original errors and stamps verbatim.
    let done = store.get_node_task("t-done").await.unwrap().unwrap();
    assert_eq!(done.status, NodeTaskStatus::Done);
    assert_eq!(done.finished_at, Some(1003));
    let cancelled = store.get_node_task("t-cancelled").await.unwrap().unwrap();
    assert_eq!(cancelled.status, NodeTaskStatus::Cancelled);
    assert_eq!(cancelled.finished_at, Some(1007));
    let err = store.get_node_task("t-err").await.unwrap().unwrap();
    assert_eq!(err.status, NodeTaskStatus::Error);
    assert_eq!(err.error.as_deref(), Some("boom"), "original error kept");
    assert_eq!(err.finished_at, Some(1010));

    // Idempotent: everything is terminal-frozen now, nothing left to do.
    let second = store
        .converge_lost_node_tasks(1600, STALE_MS)
        .await
        .unwrap();
    assert!(second.is_empty());
}

/// Exclusive boundary: exactly `stale_ms` old is still fresh (no collapse),
/// and `pending` tasks of genuinely lost nodes stay queued.
#[tokio::test]
async fn boundary_equality_and_pending_survive() {
    let store = mem().await;

    // last_seen_at == now - stale exactly: gap 400 == 400, NOT > -> fresh.
    let edge = register(&store, "node-edge", 2000 - STALE_MS).await;
    run_to_running(&store, "t-edge", &edge.id, 1995).await;

    // Genuinely lost node whose only task never got claimed.
    let gone = register(&store, "node-gone", 900).await;
    dispatch(&store, "t-pend", 4, &gone.id, 905).await;

    let converged = store
        .converge_lost_node_tasks(2000, STALE_MS)
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
