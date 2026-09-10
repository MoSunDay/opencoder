//! Brain playbook persistence — free functions over a `Connection`,
//! mirroring `brain.rs`. One row per playbook; `spec_json` is opaque to the
//! store (the brain crate owns the domain), which makes every operation a
//! single-statement read or write — no transactions needed.

use anyhow::{Context, Result};
use libsql::{Connection, Row, params};

use crate::BrainPlaybookRecord;

/// Brain playbooks (v25): the orchestration graphs (fixed or LLM-generated)
/// that fan a situation out over agent/team/dag/todos/brain executors.
pub(super) const CREATE_BRAIN_PLAYBOOKS: &str = "\
CREATE TABLE IF NOT EXISTS brain_playbooks (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  origin TEXT NOT NULL,
  situation_digest TEXT,
  spec_json TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
)";
/// Backs the "latest dynamic playbook for a situation" plan-cache probe.
pub(super) const CREATE_INDEX_BRAIN_PLAYBOOKS_DIGEST: &str = "CREATE INDEX IF NOT EXISTS idx_brain_playbooks_digest ON brain_playbooks(situation_digest, created_at)";

/// Upsert one playbook keyed by id. On conflict every field replaces EXCEPT
/// `created_at` — the original creation timestamp survives rewrites (the
/// record's own value is ignored on the update path).
pub async fn save(conn: &Connection, record: &BrainPlaybookRecord) -> Result<()> {
    conn.execute(
        "INSERT INTO brain_playbooks (id,name,origin,situation_digest,spec_json,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7) \
         ON CONFLICT(id) DO UPDATE SET name=excluded.name, origin=excluded.origin, \
         situation_digest=excluded.situation_digest, spec_json=excluded.spec_json, updated_at=excluded.updated_at",
        params![
            record.id.as_str(),
            record.name.as_str(),
            record.origin.as_str(),
            record.situation_digest.as_deref(),
            record.spec_json.as_str(),
            record.created_at,
            record.updated_at
        ],
    )
    .await
    .context("upsert brain playbook")?;
    Ok(())
}

/// Fetch one playbook by id (`None` if absent).
pub async fn get(conn: &Connection, id: &str) -> Result<Option<BrainPlaybookRecord>> {
    let mut rows = conn
        .query(
            "SELECT id,name,origin,situation_digest,spec_json,created_at,updated_at FROM brain_playbooks WHERE id=?1",
            params![id],
        )
        .await
        .context("select brain playbook")?;
    match rows.next().await? {
        Some(row) => Ok(Some(row_record(&row)?)),
        None => Ok(None),
    }
}

/// Every playbook, newest first (created_at DESC; rowid breaks
/// same-millisecond ties deterministically, mirroring the plans listing).
pub async fn list(conn: &Connection) -> Result<Vec<BrainPlaybookRecord>> {
    let mut rows = conn
        .query(
            "SELECT id,name,origin,situation_digest,spec_json,created_at,updated_at FROM brain_playbooks \
             ORDER BY created_at DESC, rowid DESC",
            (),
        )
        .await
        .context("select brain playbooks")?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        out.push(row_record(&row)?);
    }
    Ok(out)
}

/// Delete one playbook; `true` when a row was removed.
pub async fn delete(conn: &Connection, id: &str) -> Result<bool> {
    let changed = conn
        .execute("DELETE FROM brain_playbooks WHERE id=?1", params![id])
        .await
        .context("delete brain playbook")?;
    Ok(changed > 0)
}

/// Newest dynamic playbook for a situation digest — the plan-cache probe.
/// Fixed playbooks never answer this lookup (they have no digest).
pub async fn latest_by_digest(
    conn: &Connection,
    digest: &str,
) -> Result<Option<BrainPlaybookRecord>> {
    let mut rows = conn
        .query(
            "SELECT id,name,origin,situation_digest,spec_json,created_at,updated_at FROM brain_playbooks \
             WHERE situation_digest=?1 AND origin='dynamic' \
             ORDER BY created_at DESC, rowid DESC LIMIT 1",
            params![digest],
        )
        .await
        .context("select latest brain playbook")?;
    match rows.next().await? {
        Some(row) => Ok(Some(row_record(&row)?)),
        None => Ok(None),
    }
}

/// Column order shared by every brain_playbooks SELECT above.
fn row_record(row: &Row) -> Result<BrainPlaybookRecord> {
    Ok(BrainPlaybookRecord {
        id: row.get(0)?,
        name: row.get(1)?,
        origin: row.get(2)?,
        situation_digest: row.get(3)?,
        spec_json: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
    })
}
