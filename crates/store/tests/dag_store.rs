//! DAG workflow store contracts: defs upsert/read/delete, dispatch + FIFO
//! claim (pinned/unpinned, single-active-run), status transitions, cancel
//! piggyback, event stream pagination, spec snapshot, lost-node sweep.
//!
//! All clocks are explicit — no sleeping. Claim / cancel / lost-sweep mirror
//! the node_tasks protocols (see `nodes_lost_converge.rs` for the originals).

use opencoder_dag::DagRunStatus;
use opencoder_store::{DagDefRecord, DagEventRecord, DagRunRecord, LibsqlStore, NodeRecord, Store};
use serde_json::json;
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

fn def(id: &str, name: &str, spec_json: &str, at: i64) -> DagDefRecord {
    DagDefRecord {
        id: id.into(),
        name: name.into(),
        spec_json: spec_json.into(),
        created_at: at,
        updated_at: at,
    }
}

fn run(id: &str, node: Option<&str>, created_at: i64, spec_json: &str) -> DagRunRecord {
    DagRunRecord {
        id: id.into(),
        dag_id: "dag-1".into(),
        name: format!("run-{id}"),
        spec_json: spec_json.into(),
        node_id: node.map(str::to_string),
        status: DagRunStatus::Pending,
        error: None,
        created_at,
        claimed_at: None,
        finished_at: None,
    }
}

async fn dispatch(store: &LibsqlStore, id: &str, node: Option<&str>, created_at: i64) {
    store
        .dispatch_dag_run(&run(id, node, created_at, r#"{"v":1}"#))
        .await
        .unwrap();
}

// ── defs ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn defs_list_get_delete() {
    let (_dir, store) = fresh().await;
    store
        .upsert_dag_def(&def("dag-1", "build", r#"{"v":1}"#, 1_000))
        .await
        .unwrap();
    store
        .upsert_dag_def(&def("dag-2", "audit", r#"{"v":1}"#, 1_100))
        .await
        .unwrap();

    let listed = store.list_dag_defs().await.unwrap();
    let names: Vec<&str> = listed.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["audit", "build"], "catalog order is by name");

    assert_eq!(
        store.get_dag_def("dag-1").await.unwrap().unwrap().name,
        "build"
    );
    assert!(store.get_dag_def("ghost").await.unwrap().is_none());

    store.delete_dag_def("dag-1").await.unwrap();
    assert!(store.get_dag_def("dag-1").await.unwrap().is_none());
    assert_eq!(store.list_dag_defs().await.unwrap().len(), 1);
}

/// Upsert conflicts on `name`, not id: a re-publish under the same name
/// replaces the spec but keeps the FIRST row's id and created_at.
#[tokio::test]
async fn upsert_by_name_replaces_spec_keeps_id_and_created_at() {
    let (_dir, store) = fresh().await;
    store
        .upsert_dag_def(&def("dag-1", "build", r#"{"v":1}"#, 1_000))
        .await
        .unwrap();
    store
        .upsert_dag_def(&def("dag-other", "build", r#"{"v":2}"#, 2_000))
        .await
        .unwrap();

    let kept = store.get_dag_def("dag-1").await.unwrap().unwrap();
    assert_eq!(kept.spec_json, r#"{"v":2}"#);
    assert_eq!(kept.created_at, 1_000, "created_at survives the re-publish");
    assert_eq!(kept.updated_at, 2_000);
    assert!(store.get_dag_def("dag-other").await.unwrap().is_none());
    assert_eq!(store.list_dag_defs().await.unwrap().len(), 1);
}

// ── dispatch / claim ──────────────────────────────────────────────────────

/// A dispatch always enqueues fresh (status/claim bookkeeping forced clean)
/// and snapshots `spec_json` — later def edits never mutate an in-flight run.
#[tokio::test]
async fn dispatch_enqueues_fresh_and_snapshots_spec() {
    let (_dir, store) = fresh().await;
    store
        .upsert_dag_def(&def("dag-1", "build", r#"{"v":1}"#, 1_000))
        .await
        .unwrap();
    let mut dirty = run("run-1", None, 1_500, r#"{"v":1}"#);
    dirty.status = DagRunStatus::Done;
    dirty.error = Some("stale".into());
    dirty.claimed_at = Some(9);
    dirty.finished_at = Some(9);

    let stored = store.dispatch_dag_run(&dirty).await.unwrap();
    assert_eq!(stored.status, DagRunStatus::Pending);
    assert_eq!(stored.error, None);
    assert_eq!(stored.claimed_at, None);
    assert_eq!(stored.finished_at, None);

    store
        .upsert_dag_def(&def("dag-1", "build", r#"{"v":2}"#, 2_000))
        .await
        .unwrap();
    let again = store.get_dag_run("run-1").await.unwrap().unwrap();
    assert_eq!(
        again.spec_json, r#"{"v":1}"#,
        "run keeps its dispatch snapshot"
    );
    assert!(store.get_dag_run("ghost").await.unwrap().is_none());
}

/// FIFO drain order (oldest `created_at` first), single-active-run guard,
/// `node_id` stamped on an unpinned claim.
#[tokio::test]
async fn claim_is_fifo_and_single_active() {
    let (_dir, store) = fresh().await;
    let node = register(&store, "n1", 1_000).await;
    dispatch(&store, "r-newest", None, 3_000).await;
    dispatch(&store, "r-oldest", None, 1_000).await;
    dispatch(&store, "r-mid", None, 2_000).await;

    let first = store
        .claim_next_dag_run(&node.id, 4_000)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.id, "r-oldest");
    assert_eq!(first.status, DagRunStatus::Running);
    assert_eq!(first.claimed_at, Some(4_000));
    assert_eq!(first.node_id.as_deref(), Some(node.id.as_str()));

    // Single-active-run: a second claim while one is running is refused.
    assert!(store
        .claim_next_dag_run(&node.id, 4_001)
        .await
        .unwrap()
        .is_none());

    store
        .update_dag_run_status("r-oldest", DagRunStatus::Done, None, 4_100)
        .await
        .unwrap();
    let second = store
        .claim_next_dag_run(&node.id, 4_200)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(second.id, "r-mid");

    // Newest-first listing is the exact reverse of the claim order.
    let listed = store.list_dag_runs(10).await.unwrap();
    let ids: Vec<&str> = listed.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, ["r-newest", "r-mid", "r-oldest"]);
    assert_eq!(store.list_dag_runs(2).await.unwrap().len(), 2);
}

/// A run pinned to node A is invisible to node B's claim scan.
#[tokio::test]
async fn pinned_run_only_claimable_by_pinned_node() {
    let (_dir, store) = fresh().await;
    let a = register(&store, "alpha", 1_000).await;
    let b = register(&store, "beta", 1_000).await;
    dispatch(&store, "r-pin", Some(&a.id), 1_500).await;

    assert!(store
        .claim_next_dag_run(&b.id, 1_600)
        .await
        .unwrap()
        .is_none());
    let claimed = store
        .claim_next_dag_run(&a.id, 1_700)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed.id, "r-pin");
}

// ── status transitions / cancel ───────────────────────────────────────────

#[tokio::test]
async fn status_transitions_stamp_finished_and_freeze_terminal() {
    let (_dir, store) = fresh().await;
    let node = register(&store, "n1", 1_000).await;
    dispatch(&store, "r-1", None, 1_500).await;
    store.claim_next_dag_run(&node.id, 1_600).await.unwrap();

    let done = store
        .update_dag_run_status("r-1", DagRunStatus::Done, None, 2_000)
        .await
        .unwrap();
    assert_eq!(done.status, DagRunStatus::Done);
    assert_eq!(done.error, None);
    assert_eq!(
        done.finished_at,
        Some(2_000),
        "terminal write stamps finished_at"
    );

    // Terminal freeze: every move out of done bails, same-state no-ops too.
    assert!(store
        .update_dag_run_status("r-1", DagRunStatus::Running, None, 2_100)
        .await
        .is_err());
    assert!(store
        .update_dag_run_status("r-1", DagRunStatus::Done, None, 2_100)
        .await
        .is_err());

    // running -> error stores the error and stamps finished_at.
    dispatch(&store, "r-2", None, 2_200).await;
    store.claim_next_dag_run(&node.id, 2_300).await.unwrap();
    let errored = store
        .update_dag_run_status("r-2", DagRunStatus::Error, Some("boom"), 2_400)
        .await
        .unwrap();
    assert_eq!(errored.status, DagRunStatus::Error);
    assert_eq!(errored.error.as_deref(), Some("boom"));
    assert_eq!(errored.finished_at, Some(2_400));

    // Unknown ids bail with a clear message.
    let err = store
        .update_dag_run_status("ghost", DagRunStatus::Running, None, 2_500)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("not found"), "got: {err}");
}

/// `pending -> cancelled` directly (terminal stamped); `running ->
/// cancelling` surfaces through `cancelling_dag_runs` (heartbeat piggyback);
/// repeats are idempotent; terminal bails; unknown bails.
#[tokio::test]
async fn cancel_pending_direct_and_running_via_cancelling() {
    let (_dir, store) = fresh().await;
    let node = register(&store, "n1", 1_000).await;

    dispatch(&store, "r-pend", None, 1_500).await;
    store.cancel_dag_run("r-pend", 1_600).await.unwrap();
    let cancelled = store.get_dag_run("r-pend").await.unwrap().unwrap();
    assert_eq!(cancelled.status, DagRunStatus::Cancelled);
    assert_eq!(cancelled.finished_at, Some(1_600));
    assert!(
        store.cancel_dag_run("r-pend", 1_700).await.is_err(),
        "terminal bails"
    );

    dispatch(&store, "r-run", None, 1_800).await;
    store.claim_next_dag_run(&node.id, 1_900).await.unwrap();
    store.cancel_dag_run("r-run", 2_000).await.unwrap();
    let cancelling = store.get_dag_run("r-run").await.unwrap().unwrap();
    assert_eq!(cancelling.status, DagRunStatus::Cancelling);
    assert_eq!(cancelling.finished_at, None, "cancelling is not terminal");
    assert_eq!(
        store.cancelling_dag_runs(&node.id).await.unwrap(),
        ["r-run"]
    );

    store.cancel_dag_run("r-run", 2_100).await.unwrap(); // idempotent repeat
    assert!(store.cancel_dag_run("ghost", 2_100).await.is_err());
}

// ── events ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn events_assign_ascending_seqs_and_paginate_by_cursor() {
    let (_dir, store) = fresh().await;
    let events: Vec<DagEventRecord> = (0..5)
        .map(|i| DagEventRecord {
            seq: None,
            run_id: "r-1".into(),
            kind: "step_done".into(),
            step: Some(format!("s{i}")),
            payload: json!({"i": i}),
            at_ms: 1_000 + i,
        })
        .collect();
    let seqs = store.append_dag_events(&events).await.unwrap();
    assert_eq!(seqs.len(), 5);
    assert!(
        seqs.windows(2).all(|w| w[0] < w[1]),
        "seqs ascend: {seqs:?}"
    );

    let all = store.dag_events_after("r-1", 0, 100).await.unwrap();
    let steps: Vec<&str> = all.iter().map(|e| e.step.as_deref().unwrap()).collect();
    assert_eq!(steps, ["s0", "s1", "s2", "s3", "s4"]);
    assert!(all.iter().all(|e| e.seq.is_some()));
    assert_eq!(all[2].payload, json!({"i": 2}), "payload round-trips");

    // Cursor: after seqs[2] returns exactly the tail; limit is respected.
    let tail = store.dag_events_after("r-1", seqs[2], 100).await.unwrap();
    let tail_steps: Vec<&str> = tail.iter().map(|e| e.step.as_deref().unwrap()).collect();
    assert_eq!(tail_steps, ["s3", "s4"]);
    assert_eq!(store.dag_events_after("r-1", 0, 2).await.unwrap().len(), 2);
    assert!(store
        .dag_events_after("other", 0, 100)
        .await
        .unwrap()
        .is_empty());
    assert!(store.append_dag_events(&[]).await.unwrap().is_empty());
}

// ── lost-node sweep ───────────────────────────────────────────────────────

/// Heartbeat-stale nodes have their `running` AND `cancelling` runs collapsed
/// to `error("node lost")`; fresh nodes, pending rows and terminal rows stay
/// put; a re-run is a no-op.
#[tokio::test]
async fn lost_sweep_converges_running_and_cancelling() {
    let (_dir, store) = fresh().await;
    let stale = register(&store, "stale-node", 1_000).await;
    let alive = register(&store, "alive-node", 1_301).await; // gap 100 < 400

    // Stale node: one cancelling run (cancelling does not block a fresh
    // claim) plus one running run.
    dispatch(&store, "r-can", Some(&stale.id), 1_100).await;
    store.claim_next_dag_run(&stale.id, 1_150).await.unwrap();
    store.cancel_dag_run("r-can", 1_200).await.unwrap();
    dispatch(&store, "r-run", Some(&stale.id), 1_210).await;
    store.claim_next_dag_run(&stale.id, 1_250).await.unwrap();
    // Fresh node keeps its running run; an unpinned pending row waits.
    dispatch(&store, "r-live", Some(&alive.id), 1_100).await;
    store.claim_next_dag_run(&alive.id, 1_150).await.unwrap();
    dispatch(&store, "r-pend", None, 1_300).await;

    let converged = store.converge_lost_dag_runs(1_401, STALE_MS).await.unwrap();
    let mut ids: Vec<&str> = converged.iter().map(|r| r.id.as_str()).collect();
    ids.sort_unstable();
    assert_eq!(ids, ["r-can", "r-run"]);
    for r in &converged {
        assert_eq!(r.status, DagRunStatus::Error);
        assert_eq!(r.error.as_deref(), Some("node lost"));
        assert_eq!(r.finished_at, Some(1_401));
    }
    let live = store.get_dag_run("r-live").await.unwrap().unwrap();
    assert_eq!(live.status, DagRunStatus::Running);
    assert_eq!(live.finished_at, None);
    let pending = store.get_dag_run("r-pend").await.unwrap().unwrap();
    assert_eq!(pending.status, DagRunStatus::Pending);

    // Idempotent: terminal rows are frozen, so the re-sweep finds nothing.
    assert!(store
        .converge_lost_dag_runs(1_500, STALE_MS)
        .await
        .unwrap()
        .is_empty());
}
