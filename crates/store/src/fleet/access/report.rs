use super::*;

impl FleetStore {
    pub async fn pending_ids(&self, node_id: &str) -> Result<Vec<String>> {
        let node_id = node_id.to_owned();
        self.db
            .read(move |conn| async move {
                let node_id = &node_id;
                FleetConnection { conn }.pending_ids(node_id).await
            })
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
            .write(move |conn| async move {
                let node_id = &node_id;
                let records = &records;
                let pending_at_begin = pending_at_begin.as_deref();
                FleetConnection { conn }
                    .apply_index_report(node_id, records, pending_at_begin)
                    .await
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
            .write(move |conn| async move {
                let node_id = &node_id;
                let records = &records;
                let pending_at_begin = pending_at_begin.as_deref();
                let watermark = watermark.as_ref().map(|(id, n)| (id.as_str(), *n));
                FleetConnection { conn }
                    .apply_index_report_fenced(node_id, records, pending_at_begin, watermark)
                    .await
            })
            .await
    }
}
