use opencoder_core::fleet::{ExecutionIndex, ExecutionKind, ExecutionStatus};
use opencoder_store::fleet::FleetStore;

fn record(id: &str, created_at: i64, status: ExecutionStatus) -> ExecutionIndex {
    ExecutionIndex {
        id: id.into(),
        created_at,
        kind: ExecutionKind::Agent,
        node_id: "node-a".into(),
        status,
    }
}

#[tokio::test]
async fn complete_reports_allow_idle_to_running_and_multiple_batches_worth_of_rows() {
    let store = FleetStore::open_memory().await.unwrap();
    let rows: Vec<_> = (0..257)
        .map(|n| record(&format!("agent-{n}"), n, ExecutionStatus::Idle))
        .collect();
    store
        .apply_index_report("node-a", &rows, Some(&[]))
        .await
        .unwrap();
    let running = record("agent-0", 0, ExecutionStatus::Running);
    store
        .apply_index_report("node-a", &[running], None)
        .await
        .unwrap();
    assert_eq!(
        store.index("agent-0").await.unwrap().unwrap().status,
        ExecutionStatus::Running
    );
    assert_eq!(store.indexes(None, None, 500).await.unwrap().len(), 257);
}

#[tokio::test]
async fn initial_empty_report_recovers_only_pending_ids_seen_at_begin() {
    let store = FleetStore::open_memory().await.unwrap();
    store
        .put_index(&record("agent-before", 1, ExecutionStatus::Pending))
        .await
        .unwrap();
    let pending = store.pending_ids("node-a").await.unwrap();
    store
        .put_index(&record("agent-after", 2, ExecutionStatus::Pending))
        .await
        .unwrap();

    let recovered = store
        .apply_index_report("node-a", &[], Some(&pending))
        .await
        .unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].id, "agent-before");
    assert_eq!(
        store.index("agent-before").await.unwrap().unwrap().status,
        ExecutionStatus::Error
    );
    assert_eq!(
        store.index("agent-after").await.unwrap().unwrap().status,
        ExecutionStatus::Pending
    );
}

#[tokio::test]
async fn invalid_report_rolls_back_every_row_and_preserves_immutable_fields() {
    let store = FleetStore::open_memory().await.unwrap();
    store
        .put_index(&record("agent-existing", 1, ExecutionStatus::Pending))
        .await
        .unwrap();
    let mut conflict = record("agent-existing", 1, ExecutionStatus::Running);
    conflict.kind = ExecutionKind::Team;
    let error = store
        .apply_index_report(
            "node-a",
            &[record("agent-new", 2, ExecutionStatus::Idle), conflict],
            None,
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("ownership conflict"), "{error}");
    assert!(store.index("agent-new").await.unwrap().is_none());
    let existing = store.index("agent-existing").await.unwrap().unwrap();
    assert_eq!(existing.kind, ExecutionKind::Agent);
    assert_eq!(existing.status, ExecutionStatus::Pending);
}

#[tokio::test]
async fn duplicate_ids_fail_before_any_report_row_is_applied() {
    let store = FleetStore::open_memory().await.unwrap();
    let row = record("agent-duplicate", 1, ExecutionStatus::Idle);
    assert!(store
        .apply_index_report("node-a", &[row.clone(), row], None)
        .await
        .is_err());
    assert!(store.index("agent-duplicate").await.unwrap().is_none());
}
