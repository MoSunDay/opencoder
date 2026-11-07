use super::FleetStore;
use anyhow::Result;

impl FleetStore {
    pub async fn initialize_capacity(&self, max_runs: usize) -> Result<()> {
        self.db
            .write(move |store| super::initialize_capacity(store, max_runs))
            .await
    }
    pub async fn configure_capacity(&self, max_runs: usize) -> Result<()> {
        self.db
            .write(move |store| super::configure_capacity(store, max_runs))
            .await
    }
    pub async fn capacity(&self) -> Result<super::CapacitySnapshot> {
        self.db.read(super::capacity).await
    }
    pub async fn enqueue_capacity(
        &self,
        ticket: &str,
        execution: &str,
        runtime: &str,
    ) -> Result<i64> {
        let ticket = ticket.to_owned();
        let execution = execution.to_owned();
        let runtime = runtime.to_owned();
        self.db
            .write(move |store| super::enqueue_capacity(store, &ticket, &execution, &runtime))
            .await
    }
    pub async fn claim_capacity(&self, ticket: &str, runtime: &str) -> Result<bool> {
        let ticket = ticket.to_owned();
        let runtime = runtime.to_owned();
        self.db
            .write(move |store| super::claim_capacity(store, &ticket, &runtime))
            .await
    }
    pub async fn finish_capacity(&self, ticket: &str, runtime: &str) -> Result<()> {
        let ticket = ticket.to_owned();
        let runtime = runtime.to_owned();
        self.db
            .write(move |store| super::finish_capacity(store, &ticket, &runtime))
            .await
    }
    pub async fn recover_capacity(
        &self,
        ticket: &str,
        execution: &str,
        runtime: &str,
    ) -> Result<bool> {
        let ticket = ticket.to_owned();
        let execution = execution.to_owned();
        let runtime = runtime.to_owned();
        self.db
            .write(move |store| super::recover_capacity(store, &ticket, &execution, &runtime))
            .await
    }
    pub async fn runtime_tickets(&self, runtime: &str) -> Result<Vec<(String, String, String)>> {
        let runtime = runtime.to_owned();
        self.db
            .read(move |store| super::runtime_tickets(store, &runtime))
            .await
    }
}
