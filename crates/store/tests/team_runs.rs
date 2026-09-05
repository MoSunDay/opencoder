//! Functional tests for the team topic-run store API (`team_topic_runs`) —
//! the (topic, node) fan-out ledger of the opencoder-team runtime.
//!
//! Behavior contracts:
//! - upsert_list_roundtrip_and_created_at_stability: two nodes on one topic
//!   round-trip; re-upserting an existing row moves ONLY `status` (the run's
//!   `created_at` clock never restarts on refresh)
//! - finish_flips_every_row_of_the_topic: bulk finish + no-op on unknown ids
//! - node_deletion_cascades_to_its_topic_runs: FK `nodes(id) ON DELETE CASCADE`
//!
//! Runs against a real on-disk libsql file (tempdir) so WAL + FK pragmas are
//! exercised truthfully.

use opencoder_store::{
    LibsqlStore, Store, TeamTopicRunRecord, TEAM_RUN_EXECUTING, TEAM_RUN_FINISHED,
};
use tempfile::TempDir;

async fn fresh() -> (TempDir, LibsqlStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = LibsqlStore::open(dir.path().join("test.db")).await.unwrap();
    (dir, store)
}

async fn register_node(store: &LibsqlStore, name: &str) -> String {
    store
        .register_node(name, Some("v1"), Some("/tmp/wd"), None, 1_000)
        .await
        .unwrap()
        .id
}

fn rec(topic: &str, node_id: &str, status: &str, created_at: i64) -> TeamTopicRunRecord {
    TeamTopicRunRecord {
        topic_id: topic.to_string(),
        node_id: node_id.to_string(),
        status: status.to_string(),
        created_at,
    }
}

#[tokio::test]
async fn upsert_list_roundtrip_and_created_at_stability() {
    let (_dir, store) = fresh().await;
    let n1 = register_node(&store, "node-alpha").await;
    let n2 = register_node(&store, "node-beta").await;

    // Two rows, same topic, different nodes.
    store
        .upsert_team_topic_run(&rec("topic-1", &n1, TEAM_RUN_EXECUTING, 1_000))
        .await
        .unwrap();
    store
        .upsert_team_topic_run(&rec("topic-1", &n2, TEAM_RUN_EXECUTING, 1_100))
        .await
        .unwrap();

    let runs = store.list_team_topic_runs("topic-1").await.unwrap();
    assert_eq!(runs.len(), 2, "both (topic, node) pairings are listed");
    assert_eq!(runs[0].node_id, n1, "oldest created_at first");
    assert_eq!(runs[0].status, TEAM_RUN_EXECUTING);
    assert_eq!(runs[0].created_at, 1_000);
    assert_eq!(runs[1].node_id, n2);
    assert_eq!(runs[1].created_at, 1_100);

    // Re-upsert an existing row: status moves, created_at is preserved.
    store
        .upsert_team_topic_run(&rec("topic-1", &n1, TEAM_RUN_FINISHED, 9_999))
        .await
        .unwrap();
    let runs = store.list_team_topic_runs("topic-1").await.unwrap();
    assert_eq!(runs.len(), 2, "upsert replaces, never duplicates");
    assert_eq!(
        runs[0].created_at, 1_000,
        "created_at keeps its first-insert value across upserts"
    );
    assert_eq!(runs[0].status, TEAM_RUN_FINISHED, "status is refreshed");
    assert_eq!(runs[1].status, TEAM_RUN_EXECUTING, "sibling row untouched");

    // Other topics are isolated.
    let n3 = register_node(&store, "node-gamma").await;
    store
        .upsert_team_topic_run(&rec("topic-2", &n3, TEAM_RUN_EXECUTING, 1_200))
        .await
        .unwrap();
    assert_eq!(
        store.list_team_topic_runs("topic-1").await.unwrap().len(),
        2,
        "topic-2's row must not leak into topic-1"
    );
    assert_eq!(
        store.list_team_topic_runs("topic-2").await.unwrap().len(),
        1
    );
    assert_eq!(
        store
            .list_team_topic_runs("topic-unknown")
            .await
            .unwrap()
            .len(),
        0,
        "unknown topic lists empty"
    );
}

#[tokio::test]
async fn finish_flips_every_row_of_the_topic() {
    let (_dir, store) = fresh().await;
    let n1 = register_node(&store, "node-alpha").await;
    let n2 = register_node(&store, "node-beta").await;
    for (i, n) in [&n1, &n2].iter().enumerate() {
        store
            .upsert_team_topic_run(&rec("topic-fin", n, TEAM_RUN_EXECUTING, 1_000 + i as i64))
            .await
            .unwrap();
    }

    store.finish_team_topic_run("topic-fin").await.unwrap();
    let runs = store.list_team_topic_runs("topic-fin").await.unwrap();
    assert!(runs.iter().all(|r| r.status == TEAM_RUN_FINISHED));

    // Idempotent: finishing an already-finished (or unknown) topic is a no-op.
    store.finish_team_topic_run("topic-fin").await.unwrap();
    store.finish_team_topic_run("topic-nope").await.unwrap();
    assert_eq!(
        store.list_team_topic_runs("topic-fin").await.unwrap().len(),
        2
    );
}

#[tokio::test]
async fn node_deletion_cascades_to_its_topic_runs() {
    let (_dir, store) = fresh().await;
    let n1 = register_node(&store, "node-alpha").await;
    let n2 = register_node(&store, "node-beta").await;
    store
        .upsert_team_topic_run(&rec("topic-c", &n1, TEAM_RUN_EXECUTING, 1_000))
        .await
        .unwrap();
    store
        .upsert_team_topic_run(&rec("topic-c", &n2, TEAM_RUN_EXECUTING, 1_100))
        .await
        .unwrap();

    store.delete_node(&n1).await.unwrap();
    let runs = store.list_team_topic_runs("topic-c").await.unwrap();
    assert_eq!(
        runs.len(),
        1,
        "deleted node's pairing cascades away, sibling survives"
    );
    assert_eq!(runs[0].node_id, n2);
}
