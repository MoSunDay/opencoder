//! Functional tests for the schedule run store API (`schedule_runs`) — the
//! fire ledger of the control-plane cron scheduler.
//!
//! Behavior contracts:
//! - record_list_roundtrip: multiple ticks of one schedule round-trip,
//!   newest tick first, honoring the limit
//! - record_is_idempotent_per_tick: re-firing the same `(schedule_id,
//!   scheduled_for_ms)` overwrites in place (error-retry converges)
//! - last_returns_newest_tick_or_none: before any fire `None`, afterwards
//!   the latest tick; different schedules are isolated
//! - schedules_do_not_require_registered_nodes: unlike the team ledger the
//!   fire history has no FK (definitions survive node churn)
//!
//! Runs against a real on-disk libsql file (tempdir) so WAL pragmas are
//! exercised truthfully.

use opencoder_store::{
    LibsqlStore, ScheduleRunRecord, Store, SCHEDULE_RUN_ERROR, SCHEDULE_RUN_FIRED,
    SCHEDULE_RUN_MISSED,
};
use tempfile::TempDir;

async fn fresh() -> (TempDir, LibsqlStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = LibsqlStore::open(dir.path().join("test.db")).await.unwrap();
    (dir, store)
}

fn rec(schedule: &str, for_ms: i64, status: &str) -> ScheduleRunRecord {
    ScheduleRunRecord {
        schedule_id: schedule.to_string(),
        kind: "agent".into(),
        target: "act".into(),
        scheduled_for_ms: for_ms,
        fired_at_ms: for_ms + 42,
        execution_id: (status == SCHEDULE_RUN_FIRED).then(|| format!("{schedule}-{for_ms}")),
        status: status.to_string(),
        error: (status == SCHEDULE_RUN_ERROR).then(|| "submit refused".into()),
        missed: status == SCHEDULE_RUN_MISSED,
    }
}

#[tokio::test]
async fn record_list_roundtrip_newest_first_with_limit() {
    let (_dir, store) = fresh().await;
    for ms in [1_000, 2_000, 3_000] {
        store
            .record_schedule_run(&rec("nightly", ms, SCHEDULE_RUN_FIRED))
            .await
            .unwrap();
    }
    // One missed tick interleaved (older than the fired catch-up tick).
    store
        .record_schedule_run(&rec("nightly", 500, SCHEDULE_RUN_MISSED))
        .await
        .unwrap();

    let all = store.list_schedule_runs("nightly", 100).await.unwrap();
    assert_eq!(all.len(), 4);
    assert_eq!(
        all.iter().map(|r| r.scheduled_for_ms).collect::<Vec<_>>(),
        vec![3_000, 2_000, 1_000, 500],
        "newest tick first"
    );
    assert_eq!(all[0].execution_id.as_deref(), Some("nightly-3000"));
    assert_eq!(all[0].status, SCHEDULE_RUN_FIRED);
    assert_eq!(all[0].fired_at_ms, 3_042);
    assert!(all[3].missed, "skipped tick is flagged");

    let two = store.list_schedule_runs("nightly", 2).await.unwrap();
    assert_eq!(two.len(), 2);
    assert_eq!(two[0].scheduled_for_ms, 3_000);
}

#[tokio::test]
async fn record_is_idempotent_per_tick() {
    let (_dir, store) = fresh().await;
    store
        .record_schedule_run(&rec("daily", 1_000, SCHEDULE_RUN_ERROR))
        .await
        .unwrap();
    // Same tick retried after the error: converges, does not duplicate.
    store
        .record_schedule_run(&rec("daily", 1_000, SCHEDULE_RUN_FIRED))
        .await
        .unwrap();

    let runs = store.list_schedule_runs("daily", 100).await.unwrap();
    assert_eq!(runs.len(), 1, "same tick replaces in place");
    assert_eq!(runs[0].status, SCHEDULE_RUN_FIRED);
    assert!(runs[0].error.is_none());
}

#[tokio::test]
async fn last_returns_newest_tick_or_none() {
    let (_dir, store) = fresh().await;
    assert!(store.last_schedule_run("quiet").await.unwrap().is_none());

    store
        .record_schedule_run(&rec("quiet", 1_000, SCHEDULE_RUN_FIRED))
        .await
        .unwrap();
    store
        .record_schedule_run(&rec("quiet", 2_000, SCHEDULE_RUN_FIRED))
        .await
        .unwrap();
    store
        .record_schedule_run(&rec("other", 5_000, SCHEDULE_RUN_FIRED))
        .await
        .unwrap();

    let last = store.last_schedule_run("quiet").await.unwrap().unwrap();
    assert_eq!(last.scheduled_for_ms, 2_000, "latest tick wins");
    // Schedules are isolated from each other.
    assert_eq!(
        store
            .last_schedule_run("other")
            .await
            .unwrap()
            .unwrap()
            .scheduled_for_ms,
        5_000
    );
    // Unknown schedule → None, not an error.
    assert!(store.last_schedule_run("ghost").await.unwrap().is_none());
}

#[tokio::test]
async fn schedule_runs_have_no_node_fk() {
    // The team ledger cascades with nodes; the schedule ledger records
    // fires for definitions that may outlive any node, so it must accept
    // rows without any node registry entry at all.
    let (_dir, store) = fresh().await;
    store
        .record_schedule_run(&rec("no-nodes", 1_000, SCHEDULE_RUN_FIRED))
        .await
        .unwrap();
    assert!(store.last_schedule_run("no-nodes").await.unwrap().is_some());
}
