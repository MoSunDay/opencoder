use super::*;

impl FleetStore {
    pub async fn assign_runtime_inventory(&self, runtime: &str, ids: &[&str]) -> Result<()> {
        let runtime = runtime.to_owned();
        let ids = ids.iter().map(|v| (*v).to_owned()).collect::<Vec<_>>();
        self.db
            .write(move |conn| async move {
                let runtime = &runtime;
                let ids = ids.iter().map(String::as_str).collect::<Vec<_>>();
                let ids = ids.as_slice();
                FleetConnection { conn }
                    .assign_runtime_inventory(runtime, ids)
                    .await
            })
            .await
    }
}
