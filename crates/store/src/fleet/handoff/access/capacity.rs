use super::*;

impl FleetStore {
    pub async fn initialize_capacity(&self, max_runs: usize) -> Result<()> {
        self.db
            .write(move |conn| async move {
                FleetConnection { conn }.initialize_capacity(max_runs).await
            })
            .await
    }
    pub async fn configure_capacity(&self, max_runs: usize) -> Result<()> {
        self.db
            .write(move |conn| async move {
                FleetConnection { conn }.configure_capacity(max_runs).await
            })
            .await
    }
    pub async fn capacity(&self) -> Result<CapacitySnapshot> {
        self.db
            .read(move |conn| async move { FleetConnection { conn }.capacity().await })
            .await
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
            .write(move |conn| async move {
                let ticket = &ticket;
                let execution = &execution;
                let runtime = &runtime;
                FleetConnection { conn }
                    .enqueue_capacity(ticket, execution, runtime)
                    .await
            })
            .await
    }
    pub async fn claim_capacity(&self, ticket: &str, runtime: &str) -> Result<bool> {
        let ticket = ticket.to_owned();
        let runtime = runtime.to_owned();
        self.db
            .write(move |conn| async move {
                let ticket = &ticket;
                let runtime = &runtime;
                FleetConnection { conn }
                    .claim_capacity(ticket, runtime)
                    .await
            })
            .await
    }
    pub async fn finish_capacity(&self, ticket: &str, runtime: &str) -> Result<()> {
        let ticket = ticket.to_owned();
        let runtime = runtime.to_owned();
        self.db
            .write(move |conn| async move {
                let ticket = &ticket;
                let runtime = &runtime;
                FleetConnection { conn }
                    .finish_capacity(ticket, runtime)
                    .await
            })
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
            .write(move |conn| async move {
                let ticket = &ticket;
                let execution = &execution;
                let runtime = &runtime;
                FleetConnection { conn }
                    .recover_capacity(ticket, execution, runtime)
                    .await
            })
            .await
    }
    pub async fn runtime_tickets(&self, runtime: &str) -> Result<Vec<(String, String, String)>> {
        let runtime = runtime.to_owned();
        self.db
            .read(move |conn| async move {
                let runtime = &runtime;
                FleetConnection { conn }.runtime_tickets(runtime).await
            })
            .await
    }
}
