// Method group: execution-side reads and writes that sit outside the
// dedicated groups — brain layered scheduler store, reporting outbox and
// inventory/DAG snapshot domains. Adapters follow part_1_conversation:
// clone args, run the domain free-function under the write lease
// (mutations) or the lease-free read path (queries).

macro_rules! rdb_impl_execution {
    ($($methods:tt)*) => {
        rdb_impl_1! {
            $($methods)*

    // Brain v4 layered scheduler store + reporting outbox + inventory
    // domains (port group 3b). Layered commits and report acknowledgements
    // take the exclusive write lease and stage one atomic batch per call;
    // the read-side helpers are lease-free.
    async fn brain_layered(
        &self,
        id: &str,
    ) -> Result<Option<opencoder_core::brain::layered::LayeredSnapshot>> {
        let rid = id.to_string();
        self.db
            .read(move |db| crate::rdb_store::execution::brain_layered::load(db, &rid))
            .await
    }
    async fn commit_brain_layered(
        &self,
        change: &opencoder_core::brain::layered::LayeredChange,
    ) -> Result<opencoder_core::brain::layered::LayeredSnapshot> {
        let change = change.clone();
        self.db
            .write(move |db| crate::rdb_store::execution::brain_layered::commit(db, &change))
            .await
    }
    async fn brain_layered_events(
        &self,
        id: &str,
        after: u64,
        limit: u32,
    ) -> Result<Vec<opencoder_core::brain::layered::LayeredEvent>> {
        let rid = id.to_string();
        self.db
            .read(move |db| crate::rdb_store::execution::brain_layered::page(db, &rid, after, limit))
            .await
    }
    async fn reporting_recover(&self) -> Result<()> {
        self.db
            .write(|db| crate::rdb_store::execution::reporting::recover(db))
            .await
    }
    async fn reporting_format(&self, reader: u32, writer: u32) -> Result<()> {
        self.db
            .write(move |db| crate::rdb_store::execution::reporting::format(db, reader, writer))
            .await
    }
    async fn pending_reports(
        &self,
        after: Option<&str>,
        limit: u32,
    ) -> Result<Vec<crate::reporting::PendingReport>> {
        let after = after.map(str::to_string);
        self.db
            .read(move |db| crate::rdb_store::execution::reporting::pending(db, after.as_deref(), limit))
            .await
    }
    async fn has_pending_reports(&self) -> Result<bool> {
        self.db
            .read(|db| crate::rdb_store::execution::reporting::has_pending(db))
            .await
    }
    async fn report_ack(&self, ack: &crate::reporting::ReportAck) -> Result<()> {
        let ack = ack.clone();
        self.db
            .write(move |db| crate::rdb_store::execution::reporting::acknowledge(db, &ack))
            .await
    }
    async fn report_acks(&self) -> Result<Vec<crate::reporting::ReportAck>> {
        self.db
            .read(|db| crate::rdb_store::execution::reporting::acks(db))
            .await
    }
    async fn report_run_state(
        &self,
        id: &str,
    ) -> Result<Option<crate::reporting::ReportRunState>> {
        let rid = id.to_string();
        self.db
            .read(move |db| crate::rdb_store::execution::reporting::state(db, &rid))
            .await
    }
    async fn report_operation(
        &self,
        id: &str,
        operation: &str,
    ) -> Result<Option<opencoder_core::brain::layered::LayeredOperation>> {
        let rid = id.to_string();
        let op = operation.to_string();
        self.db
            .read(move |db| {
                crate::rdb_store::execution::reporting::operation(db, &rid, &op)
            })
            .await
    }
    async fn inventory_sources(
        &self,
        after: Option<u64>,
    ) -> Result<crate::inventory::InventorySources> {
        self.db
            .read(move |db| crate::rdb_store::execution::inventory::sources(db, after))
            .await
    }
    async fn dag_step_snapshot(
        &self,
        id: &str,
    ) -> Result<crate::store::dag_snapshot::DagStepSnapshot> {
        let sid = id.to_string();
        self.db
            .read(move |db| crate::rdb_store::execution::dag_snapshot::read(db, &sid))
            .await
    }
        }
    };
}
