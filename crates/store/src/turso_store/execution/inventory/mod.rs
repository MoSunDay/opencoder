//! Commit-ordered invalidation of inventory source rows, never message bodies.
mod schema;
use crate::inventory::{InventorySources, ProjectIndexSource, SessionIndexSource};
use anyhow::{Context, Result};
pub(super) use schema::initialize;
use turso::{params, Connection};

pub(super) async fn read(conn: &Connection, after: Option<u64>) -> Result<InventorySources> {
    // Database::read already owns one consistent read transaction.
    read_snapshot(conn, after).await
}

async fn read_snapshot(conn: &Connection, after: Option<u64>) -> Result<InventorySources> {
    let mut cursor = conn.query(
        "SELECT COALESCE((SELECT MIN(seq) FROM inventory_source_changes),0),COALESCE((SELECT MAX(seq) FROM inventory_source_changes),0)", ()
    ).await?;
    let row = cursor.next().await?.expect("aggregate row");
    let first = row.get::<i64>(0)? as u64;
    let revision = row.get::<i64>(1)? as u64;
    let full = after.is_none_or(|v| v > revision || v.saturating_add(1) < first);
    let mut result = InventorySources {
        revision,
        full,
        ..Default::default()
    };
    if !full && after == Some(revision) {
        return Ok(result);
    }
    let from = after.unwrap_or(0).min(i64::MAX as u64) as i64;
    let session_filter = "((s.task_type='parent' AND NOT EXISTS (SELECT 1 FROM subagent_tasks st WHERE st.child_session_id=s.id)) OR s.task_type='agent_step') AND (s.kind IS NULL OR s.kind<>'operator')";
    let sql = if full {
        format!("SELECT s.id,s.kind,s.title,s.created_at FROM sessions s WHERE {session_filter}")
    } else {
        format!("SELECT s.id,s.kind,s.title,s.created_at FROM sessions s WHERE s.id IN (SELECT id FROM inventory_source_changes WHERE source='session' AND seq>?1) AND {session_filter}")
    };
    let mut rows = if full {
        conn.query(&sql, ()).await?
    } else {
        conn.query(&sql, params![from]).await?
    };
    while let Some(row) = rows.next().await? {
        result.sessions.push(SessionIndexSource {
            id: row.get(0)?,
            kind: row.get(1)?,
            title: row.get(2)?,
            created_at: row.get(3)?,
        });
    }
    let sql = if full {
        "SELECT id,todo_id,created_at,status FROM project_todo_runs"
    } else {
        "SELECT id,todo_id,created_at,status FROM project_todo_runs WHERE id IN (SELECT id FROM inventory_source_changes WHERE source='project' AND seq>?1)"
    };
    let mut rows = if full {
        conn.query(sql, ()).await?
    } else {
        conn.query(sql, params![from]).await?
    };
    while let Some(row) = rows.next().await? {
        result.project_runs.push(ProjectIndexSource {
            id: row.get(0)?,
            todo_id: row.get(1)?,
            created_at: row.get(2)?,
            status: crate::ProjectTodoRunStatus::parse(&row.get::<String>(3)?)
                .context("invalid project inventory status")?,
        });
    }
    if !full {
        // Include changed IDs before applying their current rows. This also
        // removes rows that no longer belong to the execution inventory lane.
        let mut rows = conn
            .query(
                "SELECT DISTINCT source,id FROM inventory_source_changes WHERE seq>?1",
                params![from],
            )
            .await?;
        while let Some(row) = rows.next().await? {
            if row.get::<String>(0)? == "session" {
                result.removed_sessions.push(row.get(1)?)
            } else {
                result.removed_runs.push(row.get(1)?)
            }
        }
    }
    Ok(result)
}
