//! Validate a Runtime inventory without querying ownership once per history row.
use super::super::{FleetConnection, FleetStore};
use anyhow::{ensure, Result};
use turso::transaction::TransactionBehavior;
use turso::{params, Connection};

impl FleetConnection {
    /// Existing ownership is immutable and stays read-only. Newly discovered
    /// children are assigned atomically after rechecking under the writer lock.
    pub async fn assign_runtime_inventory(&self, runtime: &str, ids: &[&str]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let encoded = serde_json::to_string(ids)?;
        if !has_unassigned(&self.conn, runtime, &encoded).await? {
            return Ok(());
        }
        let tx = turso::transaction::Transaction::new_unchecked(
            &self.conn,
            TransactionBehavior::Immediate,
        )
        .await?;
        let mut registered = tx
            .query("SELECT 1 FROM host_runtimes WHERE id=?1", [runtime])
            .await?;
        ensure!(registered.next().await?.is_some(), "no registered runtime");
        drop(registered);
        // Another connection may have assigned an ID after the optimistic read.
        has_unassigned(&tx, runtime, &encoded).await?;
        tx.execute(
            "INSERT INTO runtime_owners(execution_id,runtime_id)
             SELECT DISTINCT requested.value,?2 FROM json_each(?1) requested
             WHERE NOT EXISTS(SELECT 1 FROM runtime_owners owner
                              WHERE owner.execution_id=requested.value)",
            params![encoded, runtime],
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }
}

async fn has_unassigned(conn: &Connection, runtime: &str, encoded: &str) -> Result<bool> {
    let mut rows = conn
        .query(
            "SELECT owner.runtime_id FROM json_each(?1) requested
             LEFT JOIN runtime_owners owner ON owner.execution_id=requested.value",
            [encoded],
        )
        .await?;
    let mut missing = false;
    while let Some(row) = rows.next().await? {
        match row.get::<Option<String>>(0)? {
            Some(owner) => ensure!(owner == runtime, "execution runtime ownership conflict"),
            None => missing = true,
        }
    }
    Ok(missing)
}

#[path = "access/inventory.rs"]
mod database_access;
#[cfg(test)]
mod tests;
