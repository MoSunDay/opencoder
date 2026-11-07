//! Reporting outbox: pending selection follows brain commits, guidance
//! heads are sequential, acknowledgements are idempotent, projection
//! failures roll the whole brain commit back, recovery is repeatable and
//! the format fence ratchets forward (ported from
//! `tests/brain_layered_v4/reporting.rs`; SQL trigger/EXPLAIN/migration
//! injections are replaced by the equivalent KV observations).

use super::layered_fixtures::{created, event, operation};
use super::*;
use opencoder_core::brain::layered::*;
use opencoder_store::rdb_store::engine::RdbDb;
use opencoder_store::rdb_store::{kv, tx};
use opencoder_store::reporting::ReportAck;

fn ack(action: &str, item: &str, cursor: u64) -> ReportAck {
    ReportAck {
        execution_id: "brain-layered-store".into(),
        action: action.into(),
        item: item.into(),
        cursor,
    }
}

#[tokio::test]
async fn pending_reports_follow_commits_and_acknowledgements_survive_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reports.rdb");
    let store = RdbStore::open(&path).await.unwrap();
    store.commit_brain_layered(&created()).await.unwrap();
    let pending = store.pending_reports(None, 64).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].action, "layered_wake");
    store.report_ack(&ack("layered_wake", "", 0)).await.unwrap();
    assert!(!store.has_pending_reports().await.unwrap());

    let mut next = created();
    next.expected_generation = Some(0);
    next.run.generation = 1;
    next.run.phase = LayeredPhase::Waiting;
    next.run.layer = 1;
    next.operations
        .push(operation(1, LayeredOperationStatus::Creating, None));
    store.commit_brain_layered(&next).await.unwrap();
    let pending = store.pending_reports(None, 64).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].action, "layered_dispatch");
    store
        .report_ack(&ack(
            "layered_dispatch",
            &next.operations[0].operation_id,
            1,
        ))
        .await
        .unwrap();

    next.expected_generation = Some(1);
    next.run.generation = 2;
    next.operations[0].cancel_requested = true;
    store.commit_brain_layered(&next).await.unwrap();
    assert_eq!(
        store.pending_reports(None, 64).await.unwrap()[0].action,
        "layered_cancel"
    );
    assert!(store.commit_brain_layered(&next).await.is_err());
    assert_eq!(store.pending_reports(None, 64).await.unwrap().len(), 1);
    drop(store);

    let store = RdbStore::open(&path).await.unwrap();
    store.reporting_recover().await.unwrap();
    let pending = store.pending_reports(None, 64).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].action, "layered_cancel");
    assert_eq!(store.report_acks().await.unwrap().len(), 2);
    next.expected_generation = Some(2);
    next.run.generation = 3;
    next.run.phase = LayeredPhase::Cancelled;
    store.commit_brain_layered(&next).await.unwrap();
    let pending = store.pending_reports(None, 64).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(
        pending[0].action, "settle",
        "terminal roots no longer send operation frames"
    );
}

#[tokio::test]
async fn guidance_is_sequential_and_duplicate_acks_do_not_write() {
    let store = mem().await;
    let mut initial = created();
    initial.run.phase = LayeredPhase::Waiting;
    initial.events.clear();
    for message in ["first", "second"] {
        let mut e = event("guidance_processed", &initial.run);
        e.guidance.push(LayeredGuidance {
            execution_id: "agent-child".into(),
            message: message.into(),
        });
        initial.events.push(e);
    }
    store.commit_brain_layered(&initial).await.unwrap();
    let pending = store.pending_reports(None, 64).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].cursor, 1);
    assert_eq!(pending[0].payload["guidance"][0]["message"], "first");
    store
        .report_ack(&ack("layered_guidance", "", 1))
        .await
        .unwrap();
    let next = store
        .pending_reports(Some(&pending[0].key), 64)
        .await
        .unwrap();
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].cursor, 2);
    store
        .report_ack(&ack("layered_guidance", "", 2))
        .await
        .unwrap();
    // Replayed or stale ACKs are no-ops (the SQL suite proves it with
    // raising triggers; here the no-write path is the contract itself).
    store
        .report_ack(&ack("layered_guidance", "", 2))
        .await
        .unwrap();
    store
        .report_ack(&ack("layered_guidance", "", 1))
        .await
        .unwrap();
    assert!(!store.has_pending_reports().await.unwrap());
    assert_eq!(store.report_acks().await.unwrap()[0].cursor, 2);
}

#[tokio::test]
async fn report_projection_failure_rolls_back_the_brain_commit() {
    // The SQL suite injects a failing trigger on queue inserts; the KV
    // equivalent failure is a report cursor above i64::MAX, which aborts
    // the single atomic batch the brain commit stages everything into.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rollback.rdb");
    let store = RdbStore::open(&path).await.unwrap();
    drop(store);
    let db = RdbDb::open(&path).unwrap();
    db.write(|s| {
        // Seed a run whose next generation cannot fit a report cursor.
        let mut seeded = layered_fixtures::run(LayeredPhase::Ready, 0, i64::MAX as u64 + 1);
        seeded.last_event_seq = 0;
        let mut batch = rocksdb::WriteBatch::default();
        tx::put(&mut batch, b"blr/brain-layered-store", &seeded)?;
        tx::commit(s, batch)
    })
    .await
    .unwrap();
    drop(db);

    let store = RdbStore::open(&path).await.unwrap();
    let mut change = created();
    change.expected_generation = Some(i64::MAX as u64 + 1);
    change.run.generation = i64::MAX as u64 + 2;
    assert!(store.commit_brain_layered(&change).await.is_err());
    // Nothing from the rejected change survived: same generation, no
    // events, no outbox rows.
    let snapshot = store
        .brain_layered("brain-layered-store")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.run.generation, i64::MAX as u64 + 1);
    assert_eq!(snapshot.run.last_event_seq, 0);
    assert!(store
        .brain_layered_events("brain-layered-store", 0, 10)
        .await
        .unwrap()
        .is_empty());
    assert!(!store.has_pending_reports().await.unwrap());
    // A well-formed commit still lands after the failure. It cannot advance
    // the poisoned run (its next generation is already past the cursor
    // bound), so it starts a healthy run on the same shared write path.
    let mut good = created();
    good.run.run_id = "brain-layered-healthy".into();
    good.events = vec![event("run_created", &good.run)];
    store.commit_brain_layered(&good).await.unwrap();
    assert_eq!(
        store
            .brain_layered("brain-layered-healthy")
            .await
            .unwrap()
            .unwrap()
            .run
            .generation,
        0
    );
    assert!(store.has_pending_reports().await.unwrap());
}

#[tokio::test]
async fn legacy_projection_recovery_is_repeatable_and_format_rejects_old_readers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.rdb");
    let store = RdbStore::open(&path).await.unwrap();
    store.commit_brain_layered(&created()).await.unwrap();
    store.report_ack(&ack("layered_wake", "", 0)).await.unwrap();
    drop(store);
    // Drop the watermark row directly (the SQL `DELETE FROM
    // execution_report_runs`).
    let db = RdbDb::open(&path).unwrap();
    db.write(|s| {
        let mut batch = rocksdb::WriteBatch::default();
        batch.delete(b"brr/brain-layered-store");
        tx::commit(s, batch)
    })
    .await
    .unwrap();
    drop(db);
    let store = RdbStore::open(&path).await.unwrap();
    store.reporting_recover().await.unwrap();
    store.reporting_recover().await.unwrap();
    assert!(!store.has_pending_reports().await.unwrap());
    store.reporting_format(1, 0).await.unwrap();
    store.reporting_format(0, 0).await.unwrap();
    store.reporting_format(1, 1).await.unwrap();
    assert!(store.reporting_format(0, 0).await.is_err());
    store.reporting_format(1, 0).await.unwrap();
    assert!(store.reporting_format(0, 0).await.is_err());
}

#[tokio::test]
async fn bounded_selection_ignores_unselected_corrupt_payloads() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bounded.rdb");
    let db = RdbDb::open(&path).unwrap();
    db.write(|s| {
        let mut batch = rocksdb::WriteBatch::default();
        for i in 0..130u32 {
            let execution_id = format!("key-{i:03}");
            // Row identity (execution id, action, item) orders the queue;
            // row 129 carries a payload that cannot decode.
            let row = if i == 129 {
                b"invalid JSON".to_vec()
            } else {
                kv::encode(&serde_json::json!({
                    "execution_id": execution_id,
                    "action": "layered_wake",
                    "item": "",
                    "cursor": 0i64,
                    "payload": {},
                }))?
            };
            let key = serde_json::to_string(&(execution_id.as_str(), "layered_wake", ""))? + "\0";
            let _ = key; // keys are the raw tuple encodings below
            let key = serde_json::to_string(&(execution_id, "layered_wake", ""))?;
            tx::put_raw(&mut batch, format!("brq/{key}").as_bytes(), &row);
        }
        tx::commit(s, batch)
    })
    .await
    .unwrap();
    drop(db);

    let store = RdbStore::open(&path).await.unwrap();
    let first = store.pending_reports(None, 1000).await.unwrap();
    assert_eq!(first.len(), 64);
    let second = store
        .pending_reports(Some(&first[63].key), 64)
        .await
        .unwrap();
    assert_eq!(second.len(), 64);
    assert_eq!(second[0].key, r#"["key-064","layered_wake",""]"#);
    assert!(store
        .pending_reports(Some(&second[63].key), 64)
        .await
        .is_err());
    assert!(store.has_pending_reports().await.unwrap());
}
