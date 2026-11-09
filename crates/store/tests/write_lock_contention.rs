//! Store-level cross-process write-lock contention: single-statement write
//! paths (session insert, token insert, schedule upsert) must queue past an
//! external write lease instead of failing instantly with `Busy`.
//!
//! An independent database open on the same file holds `BEGIN IMMEDIATE`
//! for 300 ms. Turso takes the cross-process write lock with one
//! non-blocking fcntl try, so `busy_timeout` never converts that collision
//! into waiting — only the bounded begin retry inside the write path can
//! commit past the lease.

use opencoder_core::config::{ScheduleJob, ScheduleKind, ScheduleOverlap};
use opencoder_core::identity::{token_hash, Role};
use opencoder_store::{AccessToken, SessionMeta, Store, TursoStore};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;
use tempfile::TempDir;
use turso::Connection;

async fn fresh() -> (TempDir, PathBuf, TursoStore) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("contended.db");
    let store = TursoStore::open(&path).await.unwrap();
    (dir, path, store)
}

async fn external_connection(path: &std::path::Path) -> Connection {
    let database = turso::Builder::new_local(path.to_str().unwrap())
        .experimental_multiprocess_wal(true)
        .build()
        .await
        .unwrap();
    let conn = database.connect().unwrap();
    conn.busy_timeout(Duration::from_secs(30)).unwrap();
    conn
}

/// Hold the cross-process write lease for 300 ms through a second database
/// open (the OFD locks are per open file description), then commit.
async fn external_write_lease(path: &std::path::Path) -> tokio::task::JoinHandle<()> {
    let holder = external_connection(path).await;
    holder.execute("BEGIN IMMEDIATE", ()).await.unwrap();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        holder.execute("COMMIT", ()).await.unwrap();
    })
}

#[tokio::test]
async fn create_session_queues_past_external_write_lease() {
    let (_dir, path, store) = fresh().await;
    let meta = |id: &str| SessionMeta {
        id: id.into(),
        title: Some(id.into()),
        ..Default::default()
    };
    // Bootstrap row first so the contended insert is the only write.
    store.create_session(&meta("seed")).await.unwrap();

    let lease = external_write_lease(&path).await;
    store
        .create_session(&meta("contended"))
        .await
        .expect("session insert must retry past the external lease");
    lease.await.unwrap();

    assert!(store.get_session("contended").await.unwrap().is_some());
}

#[tokio::test]
async fn token_create_queues_past_external_write_lease() {
    let (_dir, path, store) = fresh().await;
    store
        .create_user("writer", "", Role::Viewer, 1)
        .await
        .unwrap();

    let lease = external_write_lease(&path).await;
    store
        .create_access_token(
            &AccessToken {
                id: "tok-1".into(),
                user_name: "writer".into(),
                name: "contended".into(),
                created_at: 2,
                expires_at: None,
                revoked_at: None,
            },
            &token_hash("lease-token"),
        )
        .await
        .expect("token insert must retry past the external lease");
    lease.await.unwrap();

    assert_eq!(store.list_access_tokens().await.unwrap().len(), 1);
}

#[tokio::test]
async fn schedule_upsert_queues_past_external_write_lease() {
    let (_dir, path, store) = fresh().await;
    let job = ScheduleJob {
        id: "nightly".into(),
        cron: "0 3 * * *".into(),
        timezone: None,
        enabled: true,
        kind: ScheduleKind::Agent,
        target: "act".into(),
        params: BTreeMap::new(),
        overlap: ScheduleOverlap::Skip,
        node_id: None,
    };

    let lease = external_write_lease(&path).await;
    store
        .upsert_schedule(&job, 1_000)
        .await
        .expect("schedule upsert must retry past the external lease");
    lease.await.unwrap();

    let def = store.get_schedule("nightly").await.unwrap().unwrap();
    assert_eq!(def.job.id, "nightly");
    assert_eq!(def.updated_at, 1_000);
}
