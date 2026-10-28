//! Rebuild a complete peer snapshot when a coexisting Server advances the fence.
use super::*;

impl FleetStore {
    /// Apply an already reconstructed full snapshot without accepting a delta
    /// against another connection's baseline. A newer durable snapshot in the
    /// same generation supersedes it; immutable ownership is still checked.
    pub async fn apply_index_snapshot_fenced(
        &self,
        node_id: &str,
        records: &[ExecutionIndex],
        pending_at_begin: Option<&[String]>,
        watermark: (&str, u64),
    ) -> Result<Option<Vec<ExecutionIndex>>> {
        let node_id = node_id.to_owned();
        let records = records.to_vec();
        let pending = pending_at_begin.map(ToOwned::to_owned);
        let (generation, sequence) = (watermark.0.to_owned(), watermark.1);
        self.db
            .write(move |conn| async move {
                FleetConnection { conn }
                    .apply_report(
                        &node_id,
                        &records,
                        pending.as_deref(),
                        Some((&generation, sequence)),
                        None,
                        true,
                    )
                    .await
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencoder_core::fleet::{ExecutionKind, ExecutionStatus};

    #[tokio::test]
    async fn rebuilt_snapshot_respects_newer_data_and_still_validates_ownership() {
        let store = FleetStore::open_memory().await.unwrap();
        let done = ExecutionIndex {
            id: "dag-snapshot".into(),
            kind: ExecutionKind::Dag,
            node_id: "node-snapshot".into(),
            created_at: 1,
            status: ExecutionStatus::Done,
        };
        store
            .apply_index_report_committed(
                &done.node_id,
                std::slice::from_ref(&done),
                None,
                ("host-2", 20),
                None,
            )
            .await
            .unwrap();
        let running = ExecutionIndex {
            status: ExecutionStatus::Running,
            ..done.clone()
        };
        assert!(store
            .apply_index_snapshot_fenced(
                &done.node_id,
                std::slice::from_ref(&running),
                None,
                ("host-2", 10),
            )
            .await
            .unwrap()
            .is_some());
        assert_eq!(store.index(&done.id).await.unwrap().unwrap(), done);
        let conflict = ExecutionIndex {
            created_at: 2,
            ..running.clone()
        };
        assert!(store
            .apply_index_snapshot_fenced(&done.node_id, &[conflict], None, ("host-2", 10),)
            .await
            .is_err());
        assert!(store
            .apply_index_snapshot_fenced(
                &done.node_id,
                std::slice::from_ref(&running),
                None,
                ("host-1", 99),
            )
            .await
            .unwrap()
            .is_none());
        assert_eq!(store.index(&done.id).await.unwrap().unwrap(), done);
        store
            .apply_index_snapshot_fenced(
                &done.node_id,
                std::slice::from_ref(&running),
                None,
                ("host-2", 21),
            )
            .await
            .unwrap();
        assert_eq!(store.index(&done.id).await.unwrap().unwrap(), running);
    }
}
