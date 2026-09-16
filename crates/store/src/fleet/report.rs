use super::{records::read_index, FleetStore};
use anyhow::{bail, Context, Result};
use libsql::{params, Connection};
use opencoder_core::fleet::{valid_id, ExecutionIndex};
use std::collections::HashSet;

impl FleetStore {
    /// Snapshot the pending IDs eligible for first-sync recovery.
    pub async fn pending_ids(&self, node_id: &str) -> Result<Vec<String>> {
        let _guard = self.gate.lock().await;
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
        validate_report(node_id, records)?;
        let _guard = self.gate.lock().await;
        self.conn
            .execute("BEGIN IMMEDIATE", ())
            .await
            .context("begin index report transaction")?;
        match async {
            if let Some((generation, sequence)) = watermark {
                let mut rows = self.conn.query("SELECT generation,sequence FROM node_report_watermarks WHERE node_id=?1", [node_id]).await?;
                if let Some(row) = rows.next().await? {
                    let old: String = row.get(0)?;
                    let old_sequence: i64 = row.get(1)?;
                    if generation < old.as_str() || (generation == old && sequence <= old_sequence as u64) {
                        return Ok(Vec::new());
                    }
                }
                drop(rows);
                self.conn.execute("INSERT INTO node_report_watermarks VALUES (?1,?2,?3) ON CONFLICT(node_id) DO UPDATE SET generation=excluded.generation,sequence=excluded.sequence", params![node_id,generation,sequence as i64]).await?;
            }
            apply_report_tx(
            &self.conn,
            node_id,
            records,
            pending_at_begin.unwrap_or_default(),
        )
        .await
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
    pending_at_begin: &[String],
) -> Result<Vec<ExecutionIndex>> {
    let present: HashSet<_> = records.iter().map(|record| record.id.as_str()).collect();
    for record in records {
        if let Some(old) = read_index(conn, &record.id).await? {
            if old.node_id != record.node_id
                || old.created_at != record.created_at
                || old.kind != record.kind
            {
                bail!("execution ownership conflict: {}", record.id);
            }
        }
        conn.execute(
            "INSERT INTO execution_index(id,created_at,kind,node_id,status) VALUES (?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET status=excluded.status",
            params![record.id.clone(), record.created_at, record.kind.prefix(), record.node_id.clone(), record.status.as_str()],
        )
        .await?;
    }

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
