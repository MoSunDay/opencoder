use super::FleetStore;
use anyhow::Result;

impl FleetStore {
    pub async fn assign_runtime_inventory(&self, runtime: &str, ids: &[&str]) -> Result<()> {
        let runtime = runtime.to_owned();
        let ids = ids.iter().map(|v| (*v).to_owned()).collect::<Vec<_>>();
        self.db
            .write(move |store| {
                let ids = ids.iter().map(String::as_str).collect::<Vec<_>>();
                super::assign_runtime_inventory(store, &runtime, &ids)
            })
            .await
    }
}
