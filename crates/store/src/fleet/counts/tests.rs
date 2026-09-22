use super::*;
use libsql::params;
use opencoder_core::fleet::ExecutionIndex;

#[tokio::test]
async fn counts_all_active_states_and_kinds_beyond_one_page() {
    let store = FleetStore::open_memory().await.unwrap();
    assert_eq!(store.active_execution_count().await.unwrap(), 0);
    let kinds = [
        ExecutionKind::Brain,
        ExecutionKind::Agent,
        ExecutionKind::Dag,
        ExecutionKind::Team,
        ExecutionKind::Todos,
        ExecutionKind::Project,
        ExecutionKind::Maintenance,
        ExecutionKind::Operator,
        ExecutionKind::System,
    ];
    let statuses = [
        ExecutionStatus::Pending,
        ExecutionStatus::Running,
        ExecutionStatus::Idle,
        ExecutionStatus::Cancelling,
        ExecutionStatus::Interrupted,
        ExecutionStatus::Done,
        ExecutionStatus::Error,
        ExecutionStatus::Cancelled,
    ];
    let mut records = Vec::new();
    for kind in kinds {
        for status in statuses {
            for i in 0..20 {
                records.push(ExecutionIndex {
                    id: format!("{}-{}-{i}", kind.prefix(), status.as_str()),
                    created_at: i64::MAX - i,
                    kind,
                    node_id: "count-node".into(),
                    status,
                });
            }
        }
    }
    store
        .apply_index_report("count-node", &records, None)
        .await
        .unwrap();
    assert_eq!(store.active_execution_count().await.unwrap(), 720);
    // A status transition must be visible immediately, without a stale cache.
    records[0].status = ExecutionStatus::Done;
    store.put_index(&records[0]).await.unwrap();
    assert_eq!(store.active_execution_count().await.unwrap(), 719);
    records[0].status = ExecutionStatus::Idle;
    store.put_index(&records[0]).await.unwrap();
    assert_eq!(store.active_execution_count().await.unwrap(), 720);
}

#[tokio::test]
async fn rejects_unknown_kind_and_status_even_for_inactive_records() {
    for (kind, status) in [("unknown", "done"), ("dag", "unknown")] {
        let store = FleetStore::open_memory().await.unwrap();
        store
            .conn
            .execute(
                "INSERT INTO execution_index VALUES ('invalid',1,?1,'node',?2)",
                params![kind, status],
            )
            .await
            .unwrap();
        assert!(store.active_execution_count().await.is_err());
    }
}

#[tokio::test]
async fn rejects_invalid_stored_field_types_instead_of_reporting_drained() {
    for values in [
        "NULL,1,'dag','node','done'",
        "x'01',1,'dag','node','done'",
        "'invalid','bad-time','dag','node','done'",
        "'invalid',1,'dag',x'01','done'",
        "'invalid',1,x'01','node','done'",
        "'invalid',1,'dag','node',x'01'",
    ] {
        let store = FleetStore::open_memory().await.unwrap();
        store
            .conn
            .execute(
                &format!("INSERT INTO execution_index VALUES ({values})"),
                (),
            )
            .await
            .unwrap();
        assert!(store.active_execution_count().await.is_err(), "{values}");
    }
}
