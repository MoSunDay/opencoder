use super::super::FleetStore;
use libsql::{Builder, Database, TransactionBehavior};

async fn full_host() -> (tempfile::TempDir, FleetStore, Database, i64) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let store = FleetStore::open(&path).await.unwrap();
    store.initialize_capacity(1).await.unwrap();
    store
        .enqueue_capacity("running", "first", "old")
        .await
        .unwrap();
    assert!(store.claim_capacity("running", "old").await.unwrap());
    let sequence = store
        .enqueue_capacity("pending", "second", "new")
        .await
        .unwrap();
    // Contention must fail immediately if either operation attempts a write.
    store
        .conn
        .execute_batch("PRAGMA busy_timeout=0")
        .await
        .unwrap();
    let rival = Builder::new_local(&path).build().await.unwrap();
    (dir, store, rival, sequence)
}

#[tokio::test]
async fn repeated_capacity_ticket_is_read_only_under_writer_contention() {
    let (_dir, store, rival, sequence) = full_host().await;
    let connection = rival.connect().unwrap();
    let _writer = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .await
        .unwrap();
    assert_eq!(
        store
            .enqueue_capacity("pending", "second", "new")
            .await
            .unwrap(),
        sequence
    );
    assert!(store
        .enqueue_capacity("pending", "different", "new")
        .await
        .is_err());
}

#[tokio::test]
async fn full_capacity_poll_is_read_only_under_writer_contention() {
    let (_dir, store, rival, _) = full_host().await;
    let connection = rival.connect().unwrap();
    let writer = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .await
        .unwrap();
    for _ in 0..3 {
        assert!(!store.claim_capacity("pending", "new").await.unwrap());
    }
    writer.rollback().await.unwrap();
    store.finish_capacity("running", "old").await.unwrap();
    assert!(store.claim_capacity("pending", "new").await.unwrap());
    assert!(!store.claim_capacity("pending", "new").await.unwrap());
}

#[tokio::test]
async fn live_capacity_preserves_cross_runtime_fifo_after_large_completed_history() {
    let store = FleetStore::open_memory().await.unwrap();
    store.initialize_capacity(2).await.unwrap();
    store
        .conn
        .execute_batch(
            "WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<20000)
        INSERT INTO capacity_queue(ticket,execution_id,runtime_id,phase)
        SELECT 'done-ticket-'||i,'done-execution-'||i,'old','done' FROM n;",
        )
        .await
        .unwrap();
    // Execution-key order deliberately differs from FIFO sequence order.
    for (ticket, execution, runtime) in [
        ("first", "z-first", "old"),
        ("second", "m-second", "new"),
        ("third", "a-third", "old"),
    ] {
        store
            .enqueue_capacity(ticket, execution, runtime)
            .await
            .unwrap();
    }
    let tickets = store.runtime_tickets("old").await.unwrap();
    assert_eq!(
        tickets.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(),
        ["first", "third"]
    );
    let before = store.capacity().await.unwrap();
    assert_eq!((before.max_runs, before.running, before.queued), (2, 0, 3));
    assert!(!store.claim_capacity("second", "new").await.unwrap());
    assert!(store.claim_capacity("first", "old").await.unwrap());
    assert!(!store.claim_capacity("third", "old").await.unwrap());
    assert!(store.claim_capacity("second", "new").await.unwrap());
    assert!(!store.claim_capacity("third", "old").await.unwrap());
    let full = store.capacity().await.unwrap();
    assert_eq!((full.running, full.queued), (2, 1));
    store.finish_capacity("first", "old").await.unwrap();
    assert!(store.claim_capacity("third", "old").await.unwrap());
    assert_eq!(
        store.runtime_tickets("old").await.unwrap(),
        [("third".into(), "a-third".into(), "running".into())]
    );
    assert!(store.runtime_tickets("absent").await.unwrap().is_empty());
    store.finish_capacity("second", "new").await.unwrap();
    store.finish_capacity("third", "old").await.unwrap();
    let empty = store.capacity().await.unwrap();
    assert_eq!((empty.running, empty.queued), (0, 0));
    assert!(store.runtime_tickets("old").await.unwrap().is_empty());
}
