use super::*;
use opencoder_core::fleet::{ExecutionKind, ExecutionStatus};

fn record(id: &str, created_at: i64) -> ExecutionIndex {
    ExecutionIndex {
        id: id.into(),
        created_at,
        kind: ExecutionKind::Dag,
        node_id: "node-scale".into(),
        status: ExecutionStatus::Done,
    }
}

async fn changes(store: &FleetStore) -> i64 {
    store
        .db
        .write(|conn| async move {
            let mut rows = conn.query("SELECT total_changes()", ()).await?;
            Ok(rows.next().await?.unwrap().get::<i64>(0)?)
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn large_unchanged_inventory_does_not_rewrite_indexes() {
    let store = FleetStore::open_memory().await.unwrap();
    let mut records: Vec<_> = (0..12326)
        .map(|i| record(&format!("dag-scale-{i}"), i64::MAX - i))
        .collect();
    store
        .apply_index_report("node-scale", &records, None)
        .await
        .unwrap();
    let initial = changes(&store).await;
    store
        .apply_index_report("node-scale", &records, None)
        .await
        .unwrap();
    assert_eq!(
        changes(&store).await,
        initial,
        "an unchanged report must not dirty every persisted index"
    );
    records[12325].status = ExecutionStatus::Running;
    store
        .apply_index_report("node-scale", &records, None)
        .await
        .unwrap();
    assert_eq!(changes(&store).await, initial + 1);
    assert_eq!(
        store.index(&records[12325].id).await.unwrap().unwrap(),
        records[12325]
    );
    assert_eq!(
        store
            .index(&records[0].id)
            .await
            .unwrap()
            .unwrap()
            .created_at,
        i64::MAX
    );
}

#[tokio::test]
async fn report_conflict_rolls_back_watermark_and_all_new_statuses() {
    for field in ["kind", "owner", "created_at"] {
        let store = FleetStore::open_memory().await.unwrap();
        let original = record("dag-existing", 1);
        store.put_index(&original).await.unwrap();
        let mut conflict = original.clone();
        match field {
            "kind" => conflict.kind = ExecutionKind::Team,
            "owner" => {
                let mut foreign = original.clone();
                foreign.node_id = "node-other".into();
                // Use a separate ID, pre-existing under another node.
                foreign.id = "dag-foreign".into();
                store.put_index(&foreign).await.unwrap();
                conflict.id = foreign.id;
            }
            _ => conflict.created_at += 1,
        }
        let added = record("dag-added", 2);
        let stable = record("dag-status", 3);
        store.put_index(&stable).await.unwrap();
        let mut changed = stable.clone();
        changed.status = ExecutionStatus::Running;
        let error = store
            .apply_index_report_fenced(
                "node-scale",
                &[added.clone(), changed, conflict],
                None,
                Some(("host-2", 10)),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("ownership conflict"), "{error}");
        assert!(store.index(&added.id).await.unwrap().is_none());
        assert_eq!(store.index(&stable.id).await.unwrap().unwrap(), stable);
        assert_eq!(store.index(&original.id).await.unwrap().unwrap(), original);
        // The rejected generation/sequence must not suppress a valid report.
        store
            .apply_index_report_fenced(
                "node-scale",
                std::slice::from_ref(&added),
                None,
                Some(("host-2", 9)),
            )
            .await
            .unwrap();
        assert_eq!(store.index(&added.id).await.unwrap().unwrap(), added);
    }
}

#[tokio::test]
async fn delta_commit_fence_survives_other_server_reports_and_rolls_back_failures() {
    let store = FleetStore::open_memory().await.unwrap();
    let original = record("dag-delta", 1);
    store
        .apply_index_report_committed(
            "node-scale",
            std::slice::from_ref(&original),
            None,
            ("host-2", 10),
            None,
        )
        .await
        .unwrap();
    let mut changed = original.clone();
    changed.status = ExecutionStatus::Running;
    assert!(
        store
            .apply_index_report_committed(
                "node-scale",
                std::slice::from_ref(&changed),
                None,
                ("host-2", 12),
                Some(9)
            )
            .await
            .is_err()
    );
    assert_eq!(store.index(&original.id).await.unwrap().unwrap(), original);
    // A coexisting old Server advances the same durable watermark.
    store
        .apply_index_report_fenced(
            "node-scale",
            std::slice::from_ref(&original),
            None,
            Some(("host-2", 11)),
        )
        .await
        .unwrap();
    assert!(
        store
            .apply_index_report_committed(
                "node-scale",
                std::slice::from_ref(&changed),
                None,
                ("host-2", 12),
                Some(10)
            )
            .await
            .is_err()
    );
    store
        .apply_index_report_committed(
            "node-scale",
            std::slice::from_ref(&changed),
            None,
            ("host-2", 13),
            None,
        )
        .await
        .unwrap();
    let mut conflict = changed.clone();
    conflict.created_at = 99;
    assert!(
        store
            .apply_index_report_committed("node-scale", &[conflict], None, ("host-2", 14), Some(13))
            .await
            .is_err()
    );
    store
        .apply_index_report_committed("node-scale", &[], None, ("host-2", 15), Some(13))
        .await
        .unwrap();
    // Empty/removal-only deltas retain historical records and never recover missing pending IDs.
    assert_eq!(store.index(&original.id).await.unwrap().unwrap(), changed);
    assert!(
        store
            .apply_index_report_committed("node-scale", &[], Some(&[]), ("host-2", 16), Some(15))
            .await
            .is_err()
    );
    assert!(
        store
            .apply_index_report_committed("node-scale", &[], None, ("host-1", 99), None)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .apply_index_report_committed("node-scale", &[], None, ("host-2", 15), Some(15))
            .await
            .is_err()
    );
    assert!(
        store
            .apply_index_report_committed("node-scale", &[], None, ("host-2", u64::MAX), Some(15))
            .await
            .is_err()
    );
    let other = FleetStore::open_memory().await.unwrap();
    assert!(
        other
            .apply_index_report_committed("node-scale", &[], None, ("host-2", 16), Some(15))
            .await
            .is_err()
    );
}
