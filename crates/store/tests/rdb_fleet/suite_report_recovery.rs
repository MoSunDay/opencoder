//! Node index reports: pending recovery from complete inventories.
use crate::common::{assignment, prepare, request};
use crate::report_support::{agent, stored};
use opencoder_core::fleet::{ExecutionKind, ExecutionStatus};
use opencoder_store::fleet::FleetStore;
use serde_json::json;

#[tokio::test]
async fn complete_inventory_recovers_pending_ids_snapshotted_at_begin() {
    let store = FleetStore::open_memory().await.unwrap();
    store
        .put_index(&agent("agent-before", 1, ExecutionStatus::Pending))
        .await
        .unwrap();
    let pending = store.pending_ids("node-a").await.unwrap();
    store
        .put_index(&agent("agent-after", 2, ExecutionStatus::Pending))
        .await
        .unwrap();

    let recovered = store
        .apply_index_report("node-a", &[], Some(&pending))
        .await
        .unwrap();
    assert_eq!(
        recovered
            .iter()
            .map(|row| row.id.as_str())
            .collect::<Vec<_>>(),
        ["agent-before"]
    );
    assert_eq!(
        stored(&store, "agent-before").await.status,
        ExecutionStatus::Error
    );
    assert_eq!(
        stored(&store, "agent-after").await.status,
        ExecutionStatus::Pending,
        "ids observed after the snapshot are not recovered"
    );
}

#[tokio::test]
async fn prepared_outbox_entries_are_not_recovered_as_rejected() {
    let store = FleetStore::open_memory().await.unwrap();
    store
        .put_index(&agent("agent-inflight", 1, ExecutionStatus::Pending))
        .await
        .unwrap();
    let target = store.index("agent-inflight").await.unwrap().unwrap();
    let frozen = assignment(
        target,
        request(
            "agent-inflight",
            ExecutionKind::Agent,
            json!({"prompt": "once"}),
        ),
        None,
    );
    prepare(&store, &frozen).await;

    let recovered = store
        .apply_index_report("node-a", &[], Some(&["agent-inflight".to_owned()]))
        .await
        .unwrap();
    assert!(recovered.is_empty());
    assert_eq!(
        stored(&store, "agent-inflight").await.status,
        ExecutionStatus::Pending,
        "a frozen outbox entry may still be in flight"
    );
}
