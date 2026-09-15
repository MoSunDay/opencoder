use super::super::FleetStore;
use anyhow::{ensure, Context, Result};
use libsql::{params, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeRecord {
    pub id: String,
    pub release_id: String,
    pub config: Value,
    pub mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeOwner {
    pub execution_id: String,
    pub runtime_id: String,
    pub release_id: String,
}

impl FleetStore {
    pub async fn register_runtime(&self, runtime: &RuntimeRecord) -> Result<()> {
        ensure!(
            opencoder_core::fleet::valid_id(&runtime.id),
            "invalid runtime id"
        );
        ensure!(
            opencoder_core::fleet::valid_id(&runtime.release_id),
            "invalid release id"
        );
        ensure!(runtime.mode == "staged", "runtime must register staged");
        let _gate = self.gate.lock().await;
        let config = serde_json::to_string(&runtime.config)?;
        self.conn
            .execute(
                "INSERT INTO host_runtimes VALUES (?1,?2,?3,'staged') ON CONFLICT(id) DO NOTHING",
                params![
                    runtime.id.clone(),
                    runtime.release_id.clone(),
                    config.clone()
                ],
            )
            .await?;
        let mut rows = self
            .conn
            .query(
                "SELECT release_id,config FROM host_runtimes WHERE id=?1",
                [runtime.id.as_str()],
            )
            .await?;
        let row = rows.next().await?.unwrap();
        ensure!(
            row.get::<String>(0)? == runtime.release_id && row.get::<String>(1)? == config,
            "runtime identity already registered with different configuration"
        );
        Ok(())
    }

    pub async fn runtimes(&self) -> Result<Vec<RuntimeRecord>> {
        let _gate = self.gate.lock().await;
        let mut rows = self
            .conn
            .query(
                "SELECT id,release_id,config,mode FROM host_runtimes ORDER BY id",
                (),
            )
            .await?;
        let mut records = Vec::new();
        while let Some(row) = rows.next().await? {
            records.push(RuntimeRecord {
                id: row.get(0)?,
                release_id: row.get(1)?,
                config: serde_json::from_str(&row.get::<String>(2)?)?,
                mode: row.get(3)?,
            });
        }
        Ok(records)
    }

    /// Activation affects only previously unseen IDs. Ownership never moves.
    pub async fn activate_runtime(&self, id: &str) -> Result<()> {
        let _gate = self.gate.lock().await;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .await?;
        let mut rows = tx
            .query("SELECT 1 FROM host_runtimes WHERE id=?1", [id])
            .await?;
        ensure!(
            rows.next().await?.is_some(),
            "candidate runtime is not registered"
        );
        drop(rows);
        tx.execute(
            "UPDATE host_runtimes SET mode='retired' WHERE mode='active' AND id!=?1",
            [id],
        )
        .await?;
        tx.execute("UPDATE host_runtimes SET mode='active' WHERE id=?1", [id])
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn owner(&self, execution_id: &str) -> Result<Option<RuntimeOwner>> {
        let _gate = self.gate.lock().await;
        let mut rows = self.conn.query("SELECT o.runtime_id,r.release_id FROM runtime_owners o JOIN host_runtimes r ON r.id=o.runtime_id WHERE o.execution_id=?1", [execution_id]).await?;
        rows.next()
            .await?
            .map(|row| {
                Ok(RuntimeOwner {
                    execution_id: execution_id.into(),
                    runtime_id: row.get(0)?,
                    release_id: row.get(1)?,
                })
            })
            .transpose()
    }

    /// First acceptance and activation serialize in SQLite. Children explicitly
    /// inherit the reporting runtime; an existing row is never overwritten.
    pub async fn assign_runtime(
        &self,
        execution_id: &str,
        inherited: Option<&str>,
    ) -> Result<String> {
        let _gate = self.gate.lock().await;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .await?;
        let mut rows = tx
            .query(
                "SELECT runtime_id FROM runtime_owners WHERE execution_id=?1",
                [execution_id],
            )
            .await?;
        if let Some(row) = rows.next().await? {
            let runtime: String = row.get(0)?;
            ensure!(
                inherited.is_none_or(|id| id == runtime),
                "execution runtime ownership conflict"
            );
            return Ok(runtime);
        }
        drop(rows);
        let mut rows = match inherited {
            Some(id) => {
                tx.query("SELECT id FROM host_runtimes WHERE id=?1", [id])
                    .await?
            }
            None => {
                tx.query("SELECT id FROM host_runtimes WHERE mode='active'", ())
                    .await?
            }
        };
        let runtime: String = rows.next().await?.context("no active runtime")?.get(0)?;
        drop(rows);
        tx.execute(
            "INSERT INTO runtime_owners VALUES (?1,?2)",
            params![execution_id, runtime.clone()],
        )
        .await?;
        tx.commit().await?;
        Ok(runtime)
    }
}
