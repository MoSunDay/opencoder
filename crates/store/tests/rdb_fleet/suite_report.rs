//! Node index reports: five-column upserts and pending listings.
use crate::report_support::{agent, stored};
use opencoder_core::fleet::{ExecutionKind, ExecutionStatus};
use opencoder_store::fleet::FleetStore;

#[tokio::test]
async fn reports_write_all_five_columns_and_update_status_in_place() {
    let store = FleetStore::open_memory().await.unwrap();
    let rows = vec![
        agent("agent-a", 1, ExecutionStatus::Pending),
        agent("agent-b", 2, ExecutionStatus::Running),
    ];
    let recovered = store
        .apply_index_report("node-a", &rows, None)
        .await
        .unwrap();
    assert!(recovered.is_empty());
    for row in &rows {
        assert_eq!(&stored(&store, &row.id).await, row);
    }

    let updated = vec![
        agent("agent-a", 1, ExecutionStatus::Done),
        agent("agent-b", 2, ExecutionStatus::Idle),
    ];
    store
        .apply_index_report("node-a", &updated, None)
        .await
        .unwrap();
    assert_eq!(
        stored(&store, "agent-a").await.status,
        ExecutionStatus::Done
    );
    assert_eq!(
        stored(&store, "agent-b").await.status,
        ExecutionStatus::Idle
    );
    assert_eq!(stored(&store, "agent-a").await.created_at, 1);

    let listed = store
        .indexes(None, Some(ExecutionKind::Agent), 10)
        .await
        .unwrap();
    assert_eq!(
        listed.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
        ["agent-b", "agent-a"]
    );
}

#[tokio::test]
async fn pending_ids_tracks_pending_rows_per_node_sorted() {
    let store = FleetStore::open_memory().await.unwrap();
    for (offset, id) in ["agent-p2", "agent-p1"].into_iter().enumerate() {
        store
            .put_index(&agent(id, offset as i64, ExecutionStatus::Pending))
            .await
            .unwrap();
    }
    store
        .put_index(&agent("agent-run", 3, ExecutionStatus::Running))
        .await
        .unwrap();
    let mut foreign = agent("agent-other", 4, ExecutionStatus::Pending);
    foreign.node_id = "node-b".into();
    store.put_index(&foreign).await.unwrap();

    assert_eq!(
        store.pending_ids("node-a").await.unwrap(),
        ["agent-p1", "agent-p2"]
    );
    assert_eq!(store.pending_ids("node-b").await.unwrap(), ["agent-other"]);
    assert!(store.pending_ids("node-z").await.unwrap().is_empty());
}

#[tokio::test]
async fn invalid_reports_are_rejected_before_any_row_is_written() {
    let store = FleetStore::open_memory().await.unwrap();
    let duplicate = agent("agent-dup", 1, ExecutionStatus::Idle);
    assert!(store
        .apply_index_report("node-a", &[duplicate.clone(), duplicate], None)
        .await
        .is_err());
    assert!(store.index("agent-dup").await.unwrap().is_none());

    let mut foreign = agent("agent-foreign", 1, ExecutionStatus::Idle);
    foreign.node_id = "node-b".into();
    assert!(store
        .apply_index_report("node-a", &[foreign], None)
        .await
        .is_err());
    assert!(store.index("agent-foreign").await.unwrap().is_none());

    assert!(store
        .apply_index_report("node-a", &[agent("bad/id", 1, ExecutionStatus::Idle)], None)
        .await
        .is_err());
    assert!(store.apply_index_report("bad/id", &[], None).await.is_err());
}
