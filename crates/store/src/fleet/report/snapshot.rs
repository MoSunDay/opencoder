//! Rebuild a complete peer snapshot when a coexisting Server advances the fence.
use super::FleetStore;

impl FleetStore {
    /// Apply an already reconstructed full snapshot without accepting a delta
    /// against another connection's baseline. A newer durable snapshot in the
    /// same generation supersedes it; immutable ownership is still checked.
    pub async fn apply_index_snapshot_fenced(
        &self,
        node_id: &str,
        records: &[opencoder_core::fleet::ExecutionIndex],
        pending_at_begin: Option<&[String]>,
        watermark: (&str, u64),
    ) -> anyhow::Result<Option<Vec<opencoder_core::fleet::ExecutionIndex>>> {
        let node_id = node_id.to_owned();
        let records = records.to_vec();
        let pending = pending_at_begin.map(ToOwned::to_owned);
        let (generation, sequence) = (watermark.0.to_owned(), watermark.1);
        self.db
            .write(move |store| {
                super::apply_report(
                    store,
                    &node_id,
                    &records,
                    pending.as_deref(),
                    Some((&generation, sequence)),
                    None,
                    true,
                )
            })
            .await
    }
}
