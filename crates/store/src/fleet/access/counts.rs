use super::FleetStore;
use anyhow::Result;

impl FleetStore {
    pub async fn active_execution_count(&self) -> Result<u64> {
        self.db.read(super::active_execution_count).await
    }
}
