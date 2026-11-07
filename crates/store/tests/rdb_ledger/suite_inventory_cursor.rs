//! Execution-inventory change-cursor mechanics: staged changes move only
//! with committed KV batches, and pruned logs force expired cursors into a
//! full resync rebuilt straight from the source rows. Ported from
//! `tests/inventory_sources.rs`; the SQL trigger/ROLLBACK injections become
//! KV batch observations and a pruned-log head write.

use super::inventory_fixtures::session;
use super::*;
use opencoder_store::rdb_store::engine::RdbDb;
use opencoder_store::rdb_store::{execution::inventory, kv, tx};

#[tokio::test]
async fn inventory_change_log_moves_only_on_committed_batches() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("inventory-rollback.rdb");
    let store = RdbStore::open(&path).await.unwrap();
    store
        .create_session(&session("agent-test", None, 1))
        .await
        .unwrap();
    let full = store.inventory_sources(None).await.unwrap();
    assert_eq!(full.revision, 1);
    drop(store);

    // BEGIN IMMEDIATE … DELETE … ROLLBACK: everything staged in the batch is
    // dropped, so neither rows nor the change cursor move.
    let db = RdbDb::open(&path).unwrap();
    let rolled_back = db
        .write(|s| -> anyhow::Result<()> {
            let mut batch = rocksdb::WriteBatch::default();
            tx::del(&mut batch, &kv::session_key("agent-test"));
            inventory::stage(s, &mut batch, &[("session", "agent-test")])?;
            anyhow::bail!("simulated rollback");
        })
        .await;
    assert!(rolled_back.is_err());
    drop(db);

    let store = RdbStore::open(&path).await.unwrap();
    assert_eq!(
        store
            .inventory_sources(Some(full.revision))
            .await
            .unwrap()
            .revision,
        full.revision
    );
    let resynced = store.inventory_sources(None).await.unwrap();
    assert!(resynced.full);
    assert_eq!(
        resynced.sessions.len(),
        1,
        "the rolled-back delete left the row"
    );
    assert_eq!(resynced.revision, full.revision);
}

#[tokio::test]
async fn inventory_recovers_expired_cursors_and_bootstraps_existing_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("inventory-expired.rdb");
    let store = RdbStore::open(&path).await.unwrap();
    store
        .create_session(&session("agent-existing", None, 1))
        .await
        .unwrap();
    let first = store.inventory_sources(None).await.unwrap();
    assert_eq!(first.revision, 1);
    assert_eq!(first.sessions.len(), 1);
    drop(store);

    // Trigger-side retention: the log was pruned up to seq 100005 while the
    // caller still holds the old cursor, so its delta history is gone.
    let db = RdbDb::open(&path).unwrap();
    db.write(|s| {
        let mut batch = rocksdb::WriteBatch::default();
        let pruned = format!("ivc/{}/session/agent-existing", kv::pad_ord(1));
        let kept = format!("ivc/{}/session/agent-existing", kv::pad_ord(100_005));
        tx::del(&mut batch, pruned.as_bytes());
        tx::put_raw(&mut batch, kept.as_bytes(), &[]);
        tx::commit(s, batch)
    })
    .await
    .unwrap();
    drop(db);

    let reopened = RdbStore::open(&path).await.unwrap();
    let expired = reopened
        .inventory_sources(Some(first.revision))
        .await
        .unwrap();
    assert!(expired.full, "an expired cursor forces a full resync");
    assert_eq!(expired.revision, 100_005);
    assert_eq!(expired.sessions.len(), 1);
    assert_eq!(expired.sessions[0].id, "agent-existing");

    // There is no derived cursor table to bootstrap: a fresh full snapshot is
    // rebuilt straight from the source rows, and later deltas keep working.
    let initial = reopened.inventory_sources(None).await.unwrap();
    assert!(initial.full);
    assert_eq!(initial.revision, 100_005);
    reopened.delete_session("agent-existing").await.unwrap();
    let removed = reopened
        .inventory_sources(Some(initial.revision))
        .await
        .unwrap();
    assert!(!removed.full);
    assert_eq!(removed.removed_sessions, vec!["agent-existing"]);
    assert!(removed.revision > initial.revision);
}
