use super::FleetStore;
use anyhow::Result;
use serde_json::Value;

impl FleetStore {
    pub async fn catalog_reference_candidates(&self, name: &str) -> Result<Vec<(String, Value)>> {
        let name = name.to_owned();
        self.db
            .read(move |store| super::catalog_reference_candidates(store, &name))
            .await
    }
}
