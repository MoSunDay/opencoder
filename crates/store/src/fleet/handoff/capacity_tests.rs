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
