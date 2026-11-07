use super::FleetStore;
use anyhow::Result;

impl FleetStore {
    pub async fn register_runtime(&self, runtime: &super::RuntimeRecord) -> Result<()> {
        let runtime = runtime.to_owned();
        self.db
            .write(move |store| super::register_runtime(store, &runtime))
            .await
    }
    pub async fn runtimes(&self) -> Result<Vec<super::RuntimeRecord>> {
        self.db.read(super::runtimes).await
    }
    /// Activation affects only previously unseen IDs. Ownership never moves.
    pub async fn activate_runtime(&self, id: &str) -> Result<()> {
        let id = id.to_owned();
        self.db
            .write(move |store| super::activate_runtime(store, &id))
            .await
    }
    pub async fn owner(&self, execution_id: &str) -> Result<Option<super::RuntimeOwner>> {
        let execution_id = execution_id.to_owned();
        self.db
            .read(move |store| super::owner(store, &execution_id))
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
            .write(move |store| super::assign_runtime(store, &execution_id, inherited.as_deref()))
            .await
    }
}
