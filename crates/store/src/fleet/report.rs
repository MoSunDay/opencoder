use super::FleetStore;
use super::{FleetConnection, records::read_index};
use anyhow::{Context, Result, bail};
use opencoder_core::fleet::{ExecutionIndex, valid_id};
use std::collections::HashSet;
use turso::{Connection, params};

#[cfg(test)]
mod performance;
mod rows;
mod snapshot;
#[cfg(test)]
mod tests;

#[derive(Debug, thiserror::Error)]
#[error("index report baseline changed: {0}; complete snapshot required")]
pub struct IndexReportBaselineChanged(pub &'static str);

impl FleetConnection {
    /// Snapshot the pending IDs eligible for first-sync recovery.
    pub async fn pending_ids(&self, node_id: &str) -> Result<Vec<String>> {
        let mut rows = self
            .conn
            .query(
                "SELECT id FROM execution_index WHERE node_id=?1 AND status='pending' ORDER BY id",
                [node_id],
            )
            .await?;
        let mut ids = Vec::new();
        while let Some(row) = rows.next().await? {
            ids.push(row.get(0)?);
        }
        Ok(ids)
    }

    /// Apply one complete node snapshot atomically. An incomplete report never
    /// reaches this method. Recovery only considers the IDs captured at begin.
    pub async fn apply_index_report(
        &self,
        node_id: &str,
        records: &[ExecutionIndex],
        pending_at_begin: Option<&[String]>,
    ) -> Result<Vec<ExecutionIndex>> {
        self.apply_index_report_fenced(node_id, records, pending_at_begin, None)
            .await
    }

    pub async fn apply_index_report_fenced(
        &self,
        node_id: &str,
        records: &[ExecutionIndex],
        pending_at_begin: Option<&[String]>,
        watermark: Option<(&str, u64)>,
    ) -> Result<Vec<ExecutionIndex>> {
        Ok(self
            .apply_report(node_id, records, pending_at_begin, watermark, None, false)
            .await?
            .unwrap_or_default())
    }

    /// Negotiated reports must either commit at the expected base or fail.
    async fn apply_report(
        &self,
        node_id: &str,
        records: &[ExecutionIndex],
        pending_at_begin: Option<&[String]>,
        watermark: Option<(&str, u64)>,
        committed_base: Option<Option<u64>>,
        rebuilt_snapshot: bool,
    ) -> Result<Option<Vec<ExecutionIndex>>> {
        validate_report(node_id, records)?;
        // Serialize outside the shared writer gate. Whole inventories can
        // contain tens of thousands of rows during a Server handoff.
        let encoded = serde_json::to_string(records)?;
        self.conn
            .execute("BEGIN IMMEDIATE", ())
            .await
            .context("begin index report transaction")?;
        match async {
            if let Some((generation, sequence)) = watermark {
                let mut rows = self.conn.query("SELECT generation,sequence FROM node_report_watermarks WHERE node_id=?1", [node_id]).await?;
                anyhow::ensure!(sequence <= i64::MAX as u64, "index report sequence exceeds storage range");
                let previous = rows.next().await?;
                if let Some(row) = previous {
                    let old: String = row.get(0)?;
                    let old_sequence: i64 = row.get(1)?;
                    if rebuilt_snapshot && generation == old && sequence <= old_sequence as u64 {
                        drop(rows);
                        rows::validate(&self.conn, &encoded).await?;
                        return Ok(Some(Vec::new()));
                    }
                    if let Some(base) = committed_base {
                        // A retired Host must retain its socket for outstanding RPC replies.
                        // Skipping it is distinct from accepting a delta or losing its base.
                        if generation < old.as_str() { return Ok(None); }
                        if generation == old && sequence <= old_sequence as u64 {
                            return Err(IndexReportBaselineChanged("sequence superseded").into());
                        }
                        if let Some(base) = base {
                            if generation != old || base != old_sequence as u64 {
                                return Err(IndexReportBaselineChanged("committed base differs").into());
                            }
                        }
                    } else if generation < old.as_str() || (generation == old && sequence <= old_sequence as u64) {
                        return Ok(None);
                    }
                } else {
                    if matches!(committed_base, Some(Some(_))) {
                        return Err(IndexReportBaselineChanged("committed base missing").into());
                    }
                }
                drop(rows);
                self.conn.execute("INSERT INTO node_report_watermarks VALUES (?1,?2,?3) ON CONFLICT(node_id) DO UPDATE SET generation=excluded.generation,sequence=excluded.sequence", params![node_id,generation,sequence as i64]).await?;
            }
            apply_report_tx(
            &self.conn,
            node_id,
            records,
            &encoded,
            pending_at_begin.unwrap_or_default(),
        )
        .await.map(Some)
        }.await
        {
            Ok(recovered) => {
                if let Err(error) = self
                    .conn
                    .execute("COMMIT", ())
                    .await
                    .context("commit index report transaction")
                {
                    rollback(&self.conn).await;
                    return Err(error);
                }
                Ok(recovered)
            }
            Err(error) => {
                rollback(&self.conn).await;
                Err(error)
            }
        }
    }
}

fn validate_report(node_id: &str, records: &[ExecutionIndex]) -> Result<()> {
    if !valid_id(node_id) {
        bail!("invalid index report node id");
    }
    let mut ids = HashSet::with_capacity(records.len());
    for record in records {
        if !valid_id(&record.id) || record.node_id != node_id {
            bail!("invalid index report ownership: {}", record.id);
        }
        if !ids.insert(record.id.as_str()) {
            bail!("duplicate execution in index report: {}", record.id);
        }
    }
    Ok(())
}

async fn apply_report_tx(
    conn: &Connection,
    node_id: &str,
    records: &[ExecutionIndex],
    encoded: &str,
    pending_at_begin: &[String],
) -> Result<Vec<ExecutionIndex>> {
    let present: HashSet<_> = records.iter().map(|record| record.id.as_str()).collect();
    rows::apply(conn, encoded).await?;

    let mut recovered = Vec::new();
    let mut checked = HashSet::new();
    for id in pending_at_begin {
        if present.contains(id.as_str()) || !checked.insert(id.as_str()) {
            continue;
        }
        let Some(mut record) = read_index(conn, id).await? else {
            continue;
        };
        if record.node_id != node_id
            || record.status != opencoder_core::fleet::ExecutionStatus::Pending
        {
            continue;
        }
        // A prepared outbox entry may still be in flight on another server.
        // Absence from an inventory is not evidence that it was rejected.
        let mut dispatched = conn
            .query(
                "SELECT 1 FROM execution_assignments WHERE id=?1",
                [id.as_str()],
            )
            .await?;
        if dispatched.next().await?.is_some() {
            continue;
        }
        drop(dispatched);
        conn.execute(
            "UPDATE execution_index SET status='error' WHERE id=?1 AND node_id=?2 AND status='pending'",
            params![id.clone(), node_id.to_owned()],
        )
        .await?;
        record.status = opencoder_core::fleet::ExecutionStatus::Error;
        recovered.push(record);
    }
    Ok(recovered)
}

pub(super) async fn rollback(conn: &Connection) {
    if let Err(error) = conn.execute("ROLLBACK", ()).await {
        tracing::warn!(%error, "index report rollback failed");
    }
}

#[path = "access/report.rs"]
mod database_access;
