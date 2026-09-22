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
        .conn
        .query("SELECT total_changes()", ())
        .await
        .unwrap()
        .next()
        .await
        .unwrap()
        .unwrap()
        .get(0)
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
