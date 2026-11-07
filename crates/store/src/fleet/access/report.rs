use super::FleetStore;
use anyhow::Result;
use opencoder_core::fleet::ExecutionIndex;

impl FleetStore {
    pub async fn pending_ids(&self, node_id: &str) -> Result<Vec<String>> {
        let node_id = node_id.to_owned();
        self.db
            .read(move |store| super::pending_ids(store, &node_id))
            .await
    }
    pub async fn apply_index_report(
        &self,
        node_id: &str,
        records: &[ExecutionIndex],
        pending_at_begin: Option<&[String]>,
    ) -> Result<Vec<ExecutionIndex>> {
        let node_id = node_id.to_owned();
        let records = records.to_owned();
        let pending_at_begin = pending_at_begin.map(ToOwned::to_owned);
        self.db
            .write(move |store| {
                super::apply_index_report(store, &node_id, &records, pending_at_begin.as_deref())
            })
            .await
    }
    pub async fn apply_index_report_fenced(
        &self,
        node_id: &str,
        records: &[ExecutionIndex],
        pending_at_begin: Option<&[String]>,
        watermark: Option<(&str, u64)>,
    ) -> Result<Vec<ExecutionIndex>> {
        let node_id = node_id.to_owned();
        let records = records.to_owned();
        let pending_at_begin = pending_at_begin.map(ToOwned::to_owned);
        let watermark = watermark.map(|(id, n)| (id.to_owned(), n));
        self.db
            .write(move |store| {
                super::apply_index_report_fenced(
                    store,
                    &node_id,
                    &records,
                    pending_at_begin.as_deref(),
                    watermark.as_ref().map(|(id, n)| (id.as_str(), *n)),
                )
            })
            .await
    }
    pub async fn apply_index_report_committed(
        &self,
        node_id: &str,
        records: &[ExecutionIndex],
        pending_at_begin: Option<&[String]>,
        watermark: (&str, u64),
        base_sequence: Option<u64>,
    ) -> Result<Option<Vec<ExecutionIndex>>> {
        let node_id = node_id.to_owned();
        let records = records.to_vec();
        let pending = pending_at_begin.map(ToOwned::to_owned);
        let (generation, sequence) = (watermark.0.to_owned(), watermark.1);
        self.db
            .write(move |store| {
                super::apply_index_report_committed(
                    store,
                    &node_id,
                    &records,
                    pending.as_deref(),
                    (&generation, sequence),
                    base_sequence,
                )
            })
            .await
    }
}
