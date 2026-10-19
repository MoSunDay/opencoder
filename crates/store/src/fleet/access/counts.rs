use super::*;

impl FleetStore {
    pub async fn active_execution_count(&self) -> Result<u64> {
        self.db
            .read(
                move |conn| async move { FleetConnection { conn }.active_execution_count().await },
            )
            .await
    }
}
