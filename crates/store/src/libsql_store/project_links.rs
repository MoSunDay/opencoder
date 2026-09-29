use crate::project::{ProjectAssignment, ProjectAssignmentState};
use anyhow::{ensure, Result};
use libsql::{params, Connection, Row};

fn assignment(row: &Row) -> Result<ProjectAssignment> {
    Ok(ProjectAssignment {
        todo_id: row.get(0)?,
        execution_id: row.get(1)?,
        kind: row.get(2)?,
        name: row.get(3)?,
        created_at: row.get(4)?,
        result_md: row.get(5)?,
        sync_state: row.get(6)?,
    })
}

pub async fn list(conn: &Connection, todo_id: &str) -> Result<Vec<ProjectAssignment>> {
    let mut rows = conn.query(
        "SELECT todo_id,execution_id,kind,name,created_at,result_md,sync_state FROM project_todo_executions WHERE todo_id = ? ORDER BY created_at DESC, execution_id DESC",
        params![todo_id],
    ).await?;
    let mut assignments = Vec::new();
    while let Some(row) = rows.next().await? {
        assignments.push(assignment(&row)?);
    }
    Ok(assignments)
}

pub async fn latest(conn: &Connection) -> Result<Vec<ProjectAssignmentState>> {
    let mut rows = conn.query(
        "SELECT a.todo_id,a.execution_id,CASE WHEN a.result_md IS NOT NULL AND a.result_md <> '' THEN 'yes' ELSE 'no' END,a.sync_state FROM project_todo_executions a WHERE NOT EXISTS (SELECT 1 FROM project_todo_executions b WHERE b.todo_id = a.todo_id AND (b.created_at > a.created_at OR (b.created_at = a.created_at AND b.execution_id > a.execution_id)))",
        (),
    ).await?;
    let mut states = Vec::new();
    while let Some(row) = rows.next().await? {
        states.push(ProjectAssignmentState {
            todo_id: row.get(0)?,
            execution_id: row.get(1)?,
            has_result: row.get::<String>(2)? == "yes",
            sync_state: row.get(3)?,
        });
    }
    Ok(states)
}

pub async fn pending(
    conn: &Connection,
    after: &str,
    limit: usize,
) -> Result<Vec<ProjectAssignment>> {
    ensure!((1..=100).contains(&limit), "invalid assignment scan limit");
    let (after_todo, after_execution) = if after.is_empty() {
        ("", "")
    } else {
        after
            .rsplit_once(':')
            .ok_or_else(|| anyhow::anyhow!("invalid assignment cursor"))?
    };
    let mut rows = conn.query(
        "SELECT todo_id,execution_id,kind,name,created_at,result_md,sync_state FROM project_todo_executions WHERE sync_state = 'pending' AND (todo_id > ? OR (todo_id = ? AND execution_id > ?)) ORDER BY todo_id, execution_id LIMIT ?",
        params![after_todo, after_todo, after_execution, limit as i64],
    ).await?;
    let mut assignments = Vec::new();
    while let Some(row) = rows.next().await? {
        assignments.push(assignment(&row)?);
    }
    Ok(assignments)
}

pub async fn link(conn: &Connection, record: &ProjectAssignment) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO project_todo_executions (todo_id,execution_id,created_at,kind,name,result_md,sync_state) VALUES (?,?,?,?,?,?,?)",
        params![record.todo_id.as_str(), record.execution_id.as_str(), record.created_at, record.kind.as_str(), record.name.as_str(), record.result_md.as_deref(), record.sync_state.as_str()],
    ).await?;
    Ok(())
}

pub async fn finish(
    conn: &Connection,
    todo_id: &str,
    execution_id: &str,
    state: &str,
    result_md: Option<&str>,
) -> Result<()> {
    conn.execute(
        "UPDATE project_todo_executions SET sync_state = ?, result_md = ? WHERE todo_id = ? AND execution_id = ? AND sync_state = 'pending'",
        params![state, result_md, todo_id, execution_id],
    ).await?;
    Ok(())
}

pub async fn unlink(conn: &Connection, todo_id: &str, execution_id: &str) -> Result<bool> {
    Ok(conn
        .execute(
            "DELETE FROM project_todo_executions WHERE todo_id = ? AND execution_id = ?",
            params![todo_id, execution_id],
        )
        .await?
        > 0)
}
