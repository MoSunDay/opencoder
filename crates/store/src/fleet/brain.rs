use super::FleetStore;
use anyhow::{ensure, Result};
use libsql::params;
use opencoder_core::{brain::*, fleet::valid_id};
use serde_json::Value;

pub(super) async fn initialize(conn: &libsql::Connection) -> Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS brain_plan_versions (id TEXT NOT NULL, version INTEGER NOT NULL, body TEXT NOT NULL, PRIMARY KEY(id,version));").await?;
    Ok(())
}

impl FleetStore {
    /// Append-only, compare-and-swap version publication. Retrying the same
    /// exact version is idempotent; an existing version is never overwritten.
    pub async fn save_brain_plan(&self, version: &PlanVersion) -> Result<PlanDefinition> {
        self.save_brain_plan_document(&serde_json::from_value(serde_json::to_value(version)?)?)
            .await
    }

    pub async fn save_brain_plan_document(
        &self,
        version: &PlanVersion<Value>,
    ) -> Result<PlanDefinition> {
        ensure!(
            valid_id(&version.id) && version.version > 0 && version.version <= i64::MAX as u64,
            "invalid plan identity"
        );
        ensure!(
            !version.changelog.trim().is_empty(),
            "changelog is required"
        );
        let _guard = self.gate.lock().await;
        self.conn.execute("BEGIN IMMEDIATE", ()).await?;
        let result = self.save_brain_plan_tx(version).await;
        match result {
            Ok(result) => {
                if let Err(error) = self.conn.execute("COMMIT", ()).await {
                    self.conn.execute("ROLLBACK", ()).await?;
                    return Err(error.into());
                }
                Ok(result)
            }
            Err(error) => {
                self.conn.execute("ROLLBACK", ()).await?;
                Err(error)
            }
        }
    }

    async fn save_brain_plan_tx(&self, version: &PlanVersion<Value>) -> Result<PlanDefinition> {
        let previous: Option<PlanDefinition> = self
            .definition_locked("brain_plan", &version.id)
            .await?
            .map(serde_json::from_value)
            .transpose()?;
        if let Some(existing) = self
            .brain_plan_version_locked(&version.id, version.version)
            .await?
        {
            ensure!(existing == *version, "immutable plan version conflict");
            return previous.ok_or_else(|| anyhow::anyhow!("plan metadata missing"));
        }
        ensure!(
            version.version == previous.as_ref().map_or(1, |p| p.latest_version + 1),
            "plan version conflict; reload latest version"
        );
        let definition = PlanDefinition {
            id: version.id.clone(),
            title: version.plan["title"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("plan title missing"))?
                .into(),
            latest_version: version.version,
            stable_version: previous.and_then(|p| p.stable_version),
            updated_at: version.created_at,
        };
        self.conn
            .execute(
                "INSERT INTO brain_plan_versions VALUES (?1,?2,?3)",
                params![
                    version.id.clone(),
                    version.version as i64,
                    serde_json::to_string(version)?
                ],
            )
            .await?;
        self.put_definition_locked(
            "brain_plan",
            &version.id,
            &serde_json::to_value(&definition)?,
        )
        .await?;
        Ok(definition)
    }

    pub async fn brain_plan_version(&self, id: &str, version: u64) -> Result<Option<PlanVersion>> {
        self.brain_plan_document(id, version)
            .await?
            .map(|value| Ok(serde_json::from_value(serde_json::to_value(value)?)?))
            .transpose()
    }
    pub async fn brain_plan_document(
        &self,
        id: &str,
        version: u64,
    ) -> Result<Option<PlanVersion<Value>>> {
        let _guard = self.gate.lock().await;
        self.brain_plan_version_locked(id, version).await
    }
    async fn brain_plan_version_locked(
        &self,
        id: &str,
        version: u64,
    ) -> Result<Option<PlanVersion<Value>>> {
        let mut rows = self
            .conn
            .query(
                "SELECT body FROM brain_plan_versions WHERE id=?1 AND version=?2",
                params![id, version as i64],
            )
            .await?;
        rows.next()
            .await?
            .map(|row| Ok(serde_json::from_str(&row.get::<String>(0)?)?))
            .transpose()
    }

    pub async fn brain_plan_versions(
        &self,
        id: &str,
        before: Option<u64>,
    ) -> Result<Vec<PlanVersion>> {
        self.brain_plan_documents(id, before)
            .await?
            .into_iter()
            .map(|value| Ok(serde_json::from_value(serde_json::to_value(value)?)?))
            .collect()
    }
    pub async fn brain_plan_documents(
        &self,
        id: &str,
        before: Option<u64>,
    ) -> Result<Vec<PlanVersion<Value>>> {
        let _guard = self.gate.lock().await;
        let mut rows = self.conn.query("SELECT body FROM brain_plan_versions WHERE id=?1 AND (?2 IS NULL OR version<?2) ORDER BY version DESC LIMIT 20",params![id,before.map(|v|v as i64)]).await?;
        let mut result = vec![];
        while let Some(row) = rows.next().await? {
            result.push(serde_json::from_str(&row.get::<String>(0)?)?);
        }
        Ok(result)
    }

    pub async fn mark_brain_stable(&self, id: &str, version: u64) -> Result<PlanDefinition> {
        let _guard = self.gate.lock().await;
        ensure!(
            self.brain_plan_version_locked(id, version).await?.is_some(),
            "unknown plan version"
        );
        let mut definition: PlanDefinition = serde_json::from_value(
            self.definition_locked("brain_plan", id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("unknown plan"))?,
        )?;
        definition.stable_version = Some(version);
        self.put_definition_locked("brain_plan", id, &serde_json::to_value(&definition)?)
            .await?;
        Ok(definition)
    }
}
