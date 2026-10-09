//! Read the prepared receipt set before loading frozen assignment payloads.
use super::super::FleetStore;
use anyhow::Result;
use libsql::params;
use opencoder_core::fleet::Assignment;

#[cfg(test)]
mod tests;

// A normal execution's receipt key is its ID. Projects use their current run
// ID instead. Their immutable kind is already stored in execution_index by
// prepare_assignment, so only project payloads need the JSON run-ID lookup.
// CROSS JOIN fixes the lookup direction: accepted historical payloads must
// not be scanned and parsed while holding the shared Fleet writer gate.
const QUERY: &str = "
WITH pending AS MATERIALIZED (
    SELECT id FROM dispatch_receipts
    WHERE scope='execution' AND phase='prepared'
)
SELECT assignment FROM (
    SELECT a.id,a.assignment
    FROM pending p
    CROSS JOIN execution_assignments a ON a.id=p.id
    WHERE a.id>?1 AND json_extract(a.assignment,'$.request.kind') IS NOT 'project'
    UNION ALL
    SELECT a.id,a.assignment
    FROM execution_index i
    CROSS JOIN execution_assignments a ON a.id=i.id
    CROSS JOIN pending p
        ON p.id=COALESCE(json_extract(a.assignment,'$.request.input.run_id'),a.id)
    WHERE i.kind='project'
        AND json_extract(a.assignment,'$.request.kind')='project' AND a.id>?1
)
ORDER BY id LIMIT ?2";

impl FleetStore {
    /// Caller serializes operations for this subject with request_lock.
    pub async fn prepared_receipt_for(&self, scope: &str, subject: &str) -> Result<Option<String>> {
        let _gate = self.gate.lock().await;
        let mut rows = self.conn.query(
            "SELECT id FROM dispatch_receipts WHERE scope=?1 AND phase='prepared' AND json_extract(payload,'$.subject')=?2 LIMIT 1",
            params![scope, subject],
        ).await?;
        rows.next()
            .await?
            .map(|row| row.get(0).map_err(Into::into))
            .transpose()
    }

    /// Bounded recovery of durable operations outside execution admission.
    pub async fn pending_receipts(
        &self,
        scope: &str,
        after: &str,
        limit: u32,
    ) -> Result<Vec<(String, super::Receipt)>> {
        let _gate = self.gate.lock().await;
        let mut rows = self.conn.query(
            "SELECT id,fingerprint,phase,payload FROM dispatch_receipts WHERE scope=?1 AND phase='prepared' AND id>?2 ORDER BY id LIMIT ?3",
            params![scope, after, i64::from(limit.clamp(1,128))],
        ).await?;
        let mut values = Vec::new();
        while let Some(row) = rows.next().await? {
            values.push((
                row.get(0)?,
                super::Receipt {
                    fingerprint: row.get(1)?,
                    phase: row.get(2)?,
                    payload: serde_json::from_str(&row.get::<String>(3)?)?,
                },
            ));
        }
        Ok(values)
    }

    pub async fn pending_assignments(&self, after: &str, limit: u32) -> Result<Vec<Assignment>> {
        let _gate = self.gate.lock().await;
        let mut rows = self
            .conn
            .query(QUERY, params![after, i64::from(limit.min(128))])
            .await?;
        let mut assignments = Vec::new();
        while let Some(row) = rows.next().await? {
            assignments.push(serde_json::from_str(&row.get::<String>(0)?)?);
        }
        Ok(assignments)
    }
}
