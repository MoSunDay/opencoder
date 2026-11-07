use std::collections::HashSet;

use anyhow::Result;
use turso::{params, Connection};

use crate::project::InitiativeOrderConflict;

pub async fn reorder(
    conn: &Connection,
    goal_id: &str,
    expected_ids: &[String],
    ids: &[String],
    now: i64,
) -> Result<bool> {
    super::super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async {
        if !super::exists(conn, "SELECT 1 FROM project_goals WHERE id = ?1", goal_id).await? {
            return Ok(false);
        }
        let mut rows = conn.query(
            "SELECT id FROM project_initiatives WHERE goal_id = ? ORDER BY sort_key, created_at, id",
            params![goal_id],
        ).await?;
        let mut current = Vec::new();
        while let Some(row) = rows.next().await? {
            current.push(row.get::<String>(0)?);
        }
        drop(rows);
        let submitted: HashSet<_> = ids.iter().collect();
        if current != expected_ids || ids.len() != current.len()
            || submitted.len() != ids.len() || current.iter().any(|id| !submitted.contains(id))
        {
            return Err(InitiativeOrderConflict.into());
        }
        for (index, id) in ids.iter().enumerate() {
            conn.execute(
                "UPDATE project_initiatives SET sort_key = ?, updated_at = ? WHERE id = ?",
                params![i64::try_from(index)?, now, id.as_str()],
            ).await?;
        }
        Ok(true)
    }).await
}
