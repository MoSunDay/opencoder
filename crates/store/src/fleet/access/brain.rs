use super::FleetStore;
use anyhow::Result;
use opencoder_core::brain::{PlanDefinition, PlanVersion};
use serde_json::Value;

impl FleetStore {
    pub async fn delete_brain_plan(&self, id: &str) -> Result<bool> {
        let id = id.to_owned();
        self.db
            .write(move |store| super::delete_brain_plan(store, &id))
            .await
    }
    pub async fn save_brain_plan(&self, version: &PlanVersion) -> Result<PlanDefinition> {
        let document = serde_json::from_value(serde_json::to_value(version)?)
            .expect("plan version round-trips through JSON");
        self.db
            .write(move |store| super::save_brain_plan_document(store, &document))
            .await
    }
    pub async fn save_brain_plan_document(
        &self,
        version: &PlanVersion<Value>,
    ) -> Result<PlanDefinition> {
        let document = version.clone();
        self.db
            .write(move |store| super::save_brain_plan_document(store, &document))
            .await
    }
    pub async fn brain_plan_version(&self, id: &str, version: u64) -> Result<Option<PlanVersion>> {
        let id = id.to_owned();
        self.db
            .read(move |store| {
                let document = super::plan_document(store, &id, version)?;
                match document {
                    None => Ok(None),
                    Some(document) => Ok(Some(serde_json::from_value(serde_json::to_value(
                        document,
                    )?)?)),
                }
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
            .read(move |store| super::plan_document(store, &id, version))
            .await
    }
    pub async fn brain_plan_versions(
        &self,
        id: &str,
        before: Option<u64>,
    ) -> Result<Vec<PlanVersion>> {
        let id = id.to_owned();
        self.db
            .read(move |store| {
                super::plan_documents(store, &id, before)?
                    .into_iter()
                    .map(|value| Ok(serde_json::from_value(serde_json::to_value(value)?)?))
                    .collect()
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
            .read(move |store| super::plan_documents(store, &id, before))
            .await
    }
    pub async fn mark_brain_stable(&self, id: &str, version: u64) -> Result<PlanDefinition> {
        let id = id.to_owned();
        self.db
            .write(move |store| super::mark_brain_stable(store, &id, version))
            .await
    }
}
