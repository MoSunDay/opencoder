use super::*;

impl FleetStore {
    pub async fn register_runtime(&self, runtime: &RuntimeRecord) -> Result<()> {
        let runtime = runtime.to_owned();
        self.db
            .write(move |conn| async move {
                let runtime = &runtime;
                FleetConnection { conn }.register_runtime(runtime).await
            })
            .await
    }
    pub async fn runtimes(&self) -> Result<Vec<RuntimeRecord>> {
        self.db
            .read(move |conn| async move { FleetConnection { conn }.runtimes().await })
            .await
    }
    pub async fn activate_runtime(&self, id: &str) -> Result<()> {
        let id = id.to_owned();
        self.db
            .write(move |conn| async move {
                let id = &id;
                FleetConnection { conn }.activate_runtime(id).await
            })
            .await
    }
    pub async fn owner(&self, execution_id: &str) -> Result<Option<RuntimeOwner>> {
        let execution_id = execution_id.to_owned();
        self.db
            .read(move |conn| async move {
                let execution_id = &execution_id;
                FleetConnection { conn }.owner(execution_id).await
            })
            .await
    }
    pub async fn assign_runtime(
        &self,
        execution_id: &str,
        inherited: Option<&str>,
    ) -> Result<String> {
        let execution_id = execution_id.to_owned();
        let inherited = inherited.map(ToOwned::to_owned);
        self.db
            .write(move |conn| async move {
                let execution_id = &execution_id;
                let inherited = inherited.as_deref();
                FleetConnection { conn }
                    .assign_runtime(execution_id, inherited)
                    .await
            })
            .await
    }
}
