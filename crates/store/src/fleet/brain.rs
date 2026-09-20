use super::FleetStore;
use anyhow::{bail, ensure, Result};
use libsql::params;
use opencoder_core::{brain::*, fleet::valid_id};
use serde_json::Value;

pub(super) async fn initialize(conn: &libsql::Connection) -> Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS brain_plan_versions (id TEXT NOT NULL, version INTEGER NOT NULL, body TEXT NOT NULL, PRIMARY KEY(id,version));
        CREATE TABLE IF NOT EXISTS brain_resource_claims (resource TEXT NOT NULL, execution_id TEXT NOT NULL, run_id TEXT NOT NULL, mode TEXT NOT NULL, acquired INTEGER NOT NULL, released INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(resource,execution_id));
        CREATE INDEX IF NOT EXISTS brain_claim_active ON brain_resource_claims(resource,released,acquired);").await?;
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

    /// Persist both granted and waiting claims so releases can wake every
    /// blocked root. Claim sets are atomic, immutable per execution identity.
    pub async fn claim_brain_resources(
        &self,
        execution: &str,
        run: &str,
        resources: &[ResourceUse],
    ) -> Result<bool> {
        let _guard = self.gate.lock().await;
        self.conn.execute("BEGIN IMMEDIATE", ()).await?;
        let result = self
            .claim_brain_resources_tx(execution, run, resources)
            .await;
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

    async fn claim_brain_resources_tx(
        &self,
        execution: &str,
        run: &str,
        resources: &[ResourceUse],
    ) -> Result<bool> {
        let mut allowed = true;
        for resource in resources {
            let mode = if resource.mode == AccessMode::Read {
                "read"
            } else {
                "write"
            };
            let mut existing = self.conn.query("SELECT mode,run_id,released FROM brain_resource_claims WHERE resource=?1 AND execution_id=?2",params![resource.key.clone(),execution]).await?;
            if let Some(row) = existing.next().await? {
                ensure!(
                    row.get::<String>(0)? == mode
                        && row.get::<String>(1)? == run
                        && row.get::<i64>(2)? == 0,
                    "resource claim receipt conflict"
                );
            }
            let mut conflicts = self.conn.query("SELECT 1 FROM brain_resource_claims WHERE resource=?1 AND execution_id<>?2 AND acquired=1 AND released=0 AND (mode='write' OR ?3='write') LIMIT 1",params![resource.key.clone(),execution,mode]).await?;
            if conflicts.next().await?.is_some() {
                allowed = false;
            }
        }
        for resource in resources {
            self.conn.execute("INSERT INTO brain_resource_claims(resource,execution_id,run_id,mode,acquired) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(resource,execution_id) DO UPDATE SET acquired=excluded.acquired",params![resource.key.clone(),execution,run,if resource.mode == AccessMode::Read { "read" } else { "write" },allowed as i64]).await?;
        }
        Ok(allowed)
    }

    /// Call only after a definitive terminal receipt (or rejected admission).
    /// Rows are retained as the audit trail; claims never expire by wall time.
    pub async fn release_brain_resources(&self, execution: &str) -> Result<Vec<String>> {
        let _guard = self.gate.lock().await;
        self.conn
            .execute(
                "UPDATE brain_resource_claims SET released=1 WHERE execution_id=?1",
                [execution],
            )
            .await?;
        let mut rows = self.conn.query("SELECT DISTINCT run_id FROM brain_resource_claims WHERE acquired=0 AND released=0 ORDER BY run_id",()).await?;
        let mut runs = vec![];
        while let Some(row) = rows.next().await? {
            runs.push(row.get(0)?);
        }
        Ok(runs)
    }

    pub async fn brain_resource_claims(&self, run: &str) -> Result<Vec<ResourceClaim>> {
        let _guard = self.gate.lock().await;
        let mut rows = self.conn.query("SELECT resource,mode,execution_id,released FROM brain_resource_claims WHERE run_id=?1 ORDER BY resource,execution_id",[run]).await?;
        let mut result = vec![];
        while let Some(row) = rows.next().await? {
            let mode = match row.get::<String>(1)?.as_str() {
                "read" => AccessMode::Read,
                "write" => AccessMode::Write,
                _ => bail!("invalid resource claim mode"),
            };
            result.push(ResourceClaim {
                resource: row.get(0)?,
                mode,
                execution_id: row.get(2)?,
                run_id: run.into(),
                released: row.get::<i64>(3)? != 0,
            });
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn unrelated_definition_write_cannot_join_a_rolled_back_plan_transaction() {
        let store = std::sync::Arc::new(FleetStore::open_memory().await.unwrap());
        let guard = store.gate.lock().await;
        store.conn.execute("BEGIN IMMEDIATE", ()).await.unwrap();
        let other = store.clone();
        let mut write = tokio::spawn(async move {
            other
                .put_definition(
                    "team",
                    "independent",
                    &serde_json::json!({"name":"independent"}),
                )
                .await
        });
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(30), &mut write)
                .await
                .is_err()
        );
        store.conn.execute("ROLLBACK", ()).await.unwrap();
        drop(guard);
        write.await.unwrap().unwrap();
        assert!(store
            .definition("team", "independent")
            .await
            .unwrap()
            .is_some());
    }
}
