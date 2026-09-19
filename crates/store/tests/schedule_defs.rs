//! Functional tests for the schedule DEFINITION store API (`schedules`,
//! schema v27) — the control-plane cron scheduler's source of truth.
//!
//! Behavior contracts:
//! - upsert_get_roundtrip: the `ScheduleJob` body (incl. params JSON,
//!   timezone, overlap, node pin) round-trips untouched
//! - upsert_is_idempotent_on_id: a re-upsert updates the body and
//!   `updated_at` while `created_at` stays at the first insert
//! - list_is_stable_id_order: listing does not depend on insert history
//! - delete_keeps_fire_history: the `schedule_runs` ledger outlives its
//!   definition (no FK), and `last_schedule_run` keeps answering
//! - fresh bootstrap creates the table; reopening is idempotent
//!
//! Runs against a real on-disk libsql file (tempdir) so WAL pragmas are
//! exercised truthfully.

use opencoder_core::config::{ScheduleJob, ScheduleKind, ScheduleOverlap};
use opencoder_store::{LibsqlStore, Store, SCHEDULE_RUN_FIRED};
use std::collections::BTreeMap;
use tempfile::TempDir;

async fn fresh() -> (TempDir, LibsqlStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = LibsqlStore::open(dir.path().join("test.db")).await.unwrap();
    (dir, store)
}

fn job(id: &str) -> ScheduleJob {
    let mut params = BTreeMap::new();
    params.insert("prompt".to_string(), serde_json::json!("ping {{now:%Y}}"));
    ScheduleJob {
        id: id.to_string(),
        cron: "0 3 * * *".to_string(),
        timezone: Some("+08:00".to_string()),
        enabled: true,
        kind: ScheduleKind::Agent,
        target: "act".to_string(),
        params,
        overlap: ScheduleOverlap::Skip,
        node_id: Some("node-a".to_string()),
    }
}

#[tokio::test]
async fn upsert_get_roundtrips_the_job_body() {
    let (_dir, store) = fresh().await;
    store.upsert_schedule(&job("nightly"), 1_000).await.unwrap();

    let def = store.get_schedule("nightly").await.unwrap().unwrap();
    assert_eq!(def.job, job("nightly"));
    assert_eq!(def.created_at, 1_000);
    assert_eq!(def.updated_at, 1_000);

    // Every definition field survives the store boundary, including the
    // params map the fire path renders through `render_params`.
    let listed = store.list_schedules().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(
        listed[0].job.params.get("prompt").unwrap(),
        def.job.params.get("prompt").unwrap()
    );
}

#[tokio::test]
async fn upsert_preserves_created_at_and_bumps_updated_at() {
    let (_dir, store) = fresh().await;
    store.upsert_schedule(&job("nightly"), 1_000).await.unwrap();
    let mut next = job("nightly");
    next.cron = "*/5 * * * *".to_string();
    next.overlap = ScheduleOverlap::Allow;
    next.node_id = None;
    store.upsert_schedule(&next, 9_000).await.unwrap();

    let def = store.get_schedule("nightly").await.unwrap().unwrap();
    assert_eq!(def.job.cron, "*/5 * * * *");
    assert_eq!(def.job.overlap, ScheduleOverlap::Allow);
    assert_eq!(def.job.node_id, None);
    assert_eq!(def.created_at, 1_000, "creation metadata is server-owned");
    assert_eq!(def.updated_at, 9_000);
    assert_eq!(
        store.list_schedules().await.unwrap().len(),
        1,
        "upsert, not duplicate"
    );
}

#[tokio::test]
async fn list_is_stable_id_order() {
    let (_dir, store) = fresh().await;
    for id in ["zebra", "alpha", "midway"] {
        store.upsert_schedule(&job(id), 1_000).await.unwrap();
    }
    let ids: Vec<String> = store
        .list_schedules()
        .await
        .unwrap()
        .into_iter()
        .map(|def| def.job.id)
        .collect();
    assert_eq!(ids, vec!["alpha", "midway", "zebra"]);
}

#[tokio::test]
async fn delete_removes_the_definition_but_keeps_the_fire_ledger() {
    let (_dir, store) = fresh().await;
    store.upsert_schedule(&job("nightly"), 1_000).await.unwrap();
    store
        .record_schedule_run(&opencoder_store::ScheduleRunRecord {
            schedule_id: "nightly".into(),
            kind: "agent".into(),
            target: "act".into(),
            scheduled_for_ms: 2_000,
            fired_at_ms: 2_000,
            execution_id: Some("agent-nightly-2000".into()),
            status: SCHEDULE_RUN_FIRED.into(),
            error: None,
            missed: false,
        })
        .await
        .unwrap();

    store.delete_schedule("nightly").await.unwrap();
    assert!(store.get_schedule("nightly").await.unwrap().is_none());
    assert!(store.list_schedules().await.unwrap().is_empty());
    // No FK: the ledger outlives the definition (audit trail).
    let runs = store.list_schedule_runs("nightly", 10).await.unwrap();
    assert_eq!(runs.len(), 1, "fire history outlives the definition");
}

#[tokio::test]
async fn bootstrap_creates_the_table_and_reopens_idempotently() {
    let (dir, store) = fresh().await;
    let conn = store.conn().await.unwrap();
    let stmt = conn
        .prepare("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='schedules'")
        .await
        .unwrap();
    let mut rows = stmt.query(()).await.unwrap();
    let n = rows
        .next()
        .await
        .unwrap()
        .expect("COUNT row")
        .get::<i64>(0)
        .unwrap();
    assert_eq!(n, 1, "the schedules table must exist after bootstrap");
    drop(store);

    // Reopening (idempotent bootstrap) must not fail nor drop definitions.
    let store = LibsqlStore::open(dir.path().join("test.db")).await.unwrap();
    store
        .upsert_schedule(&job("survives"), 1_000)
        .await
        .unwrap();
    drop(store);
    let store = LibsqlStore::open(dir.path().join("test.db")).await.unwrap();
    assert_eq!(store.list_schedules().await.unwrap().len(), 1);
}
