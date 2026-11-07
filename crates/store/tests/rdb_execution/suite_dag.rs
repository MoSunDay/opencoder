//! DAG contracts, part 1 (defs + dispatch + claim), ported from
//! `tests/dag_store.rs`: definition CRUD with name-conflict upsert
//! semantics, dispatch snapshotting, FIFO claiming with the single-active
//! guard, pinned runs and the cancelling hold.

use super::*;

// ── defs ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn defs_list_get_delete() {
    let store = mem().await;
    store
        .upsert_dag_def(&dag_def("dag-1", "build", r#"{"v":1}"#, 1_000))
        .await
        .unwrap();
    store
        .upsert_dag_def(&dag_def("dag-2", "audit", r#"{"v":1}"#, 1_100))
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
    let store = mem().await;
    store
        .upsert_dag_def(&dag_def("dag-1", "build", r#"{"v":1}"#, 1_000))
        .await
        .unwrap();
    store
        .upsert_dag_def(&dag_def("dag-other", "build", r#"{"v":2}"#, 2_000))
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
    let store = mem().await;
    store
        .upsert_dag_def(&dag_def("dag-1", "build", r#"{"v":1}"#, 1_000))
        .await
        .unwrap();
    let mut dirty = dag_run("run-1", None, 1_500, r#"{"v":1}"#);
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
        .upsert_dag_def(&dag_def("dag-1", "build", r#"{"v":2}"#, 2_000))
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
    let store = mem().await;
    let node = register(&store, "n1", 1_000).await;
    dispatch_dag(&store, "r-newest", None, 3_000).await;
    dispatch_dag(&store, "r-oldest", None, 1_000).await;
    dispatch_dag(&store, "r-mid", None, 2_000).await;

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
    let store = mem().await;
    let a = register(&store, "alpha", 1_000).await;
    let b = register(&store, "beta", 1_000).await;
    dispatch_dag(&store, "r-pin", Some(&a.id), 1_500).await;

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

/// A `cancelling` run still holds the node's single-active slot: a
/// mid-cancel node cannot start a second run until the current one lands
/// terminal and frees the slot.
#[tokio::test]
async fn claim_blocked_while_cancelling() {
    let store = mem().await;
    let node = register(&store, "n1", 1_000).await;
    dispatch_dag(&store, "r-1", None, 1_500).await;
    dispatch_dag(&store, "r-2", None, 1_600).await;

    let first = store
        .claim_next_dag_run(&node.id, 1_700)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.id, "r-1");
    store.cancel_dag_run("r-1", 1_800).await.unwrap();
    assert_eq!(
        store.get_dag_run("r-1").await.unwrap().unwrap().status,
        DagRunStatus::Cancelling
    );

    assert!(
        store
            .claim_next_dag_run(&node.id, 1_900)
            .await
            .unwrap()
            .is_none(),
        "cancelling must block a fresh claim"
    );

    store
        .update_dag_run_status("r-1", DagRunStatus::Cancelled, None, 2_000)
        .await
        .unwrap();
    let second = store
        .claim_next_dag_run(&node.id, 2_100)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(second.id, "r-2");
}
