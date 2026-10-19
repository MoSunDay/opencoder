use super::*;

impl FleetStore {
    pub async fn delete_brain_plan(&self, id: &str) -> Result<bool> {
        let id = id.to_owned();
        self.db
            .write(move |conn| async move {
                let id = &id;
                FleetConnection { conn }.delete_brain_plan(id).await
            })
            .await
    }
    pub async fn save_brain_plan(&self, version: &PlanVersion) -> Result<PlanDefinition> {
        let version = version.to_owned();
        self.db
            .write(move |conn| async move {
                let version = &version;
                FleetConnection { conn }.save_brain_plan(version).await
            })
            .await
    }
    pub async fn save_brain_plan_document(
        &self,
        version: &PlanVersion<Value>,
    ) -> Result<PlanDefinition> {
        let version = version.to_owned();
        self.db
            .write(move |conn| async move {
                let version = &version;
                FleetConnection { conn }
                    .save_brain_plan_document(version)
                    .await
            })
            .await
    }
    pub async fn brain_plan_version(&self, id: &str, version: u64) -> Result<Option<PlanVersion>> {
        let id = id.to_owned();
        self.db
            .read(move |conn| async move {
                let id = &id;
                FleetConnection { conn }
                    .brain_plan_version(id, version)
                    .await
            })
            .await
    }
    pub async fn brain_plan_document(
        &self,
        id: &str,
        version: u64,
    ) -> Result<Option<PlanVersion<Value>>> {
        let id = id.to_owned();
        self.db
            .read(move |conn| async move {
                let id = &id;
                FleetConnection { conn }
                    .brain_plan_document(id, version)
                    .await
            })
            .await
    }
    pub async fn brain_plan_versions(
        &self,
        id: &str,
        before: Option<u64>,
    ) -> Result<Vec<PlanVersion>> {
        let id = id.to_owned();
        self.db
            .read(move |conn| async move {
                let id = &id;
                FleetConnection { conn }
                    .brain_plan_versions(id, before)
                    .await
            })
            .await
    }
    pub async fn brain_plan_documents(
        &self,
        id: &str,
        before: Option<u64>,
    ) -> Result<Vec<PlanVersion<Value>>> {
        let id = id.to_owned();
        self.db
            .read(move |conn| async move {
                let id = &id;
                FleetConnection { conn }
                    .brain_plan_documents(id, before)
                    .await
            })
            .await
    }
    pub async fn mark_brain_stable(&self, id: &str, version: u64) -> Result<PlanDefinition> {
        let id = id.to_owned();
        self.db
            .write(move |conn| async move {
                let id = &id;
                FleetConnection { conn }
                    .mark_brain_stable(id, version)
                    .await
            })
            .await
    }
}
