use crate::project::ProjectAssignment;
use anyhow::Result;
use libsql::{params, Connection, Row};

fn assignment(row: &Row) -> Result<ProjectAssignment> {
    Ok(ProjectAssignment {
        todo_id: row.get(0)?,
        execution_id: row.get(1)?,
        kind: row.get(2)?,
        name: row.get(3)?,
        created_at: row.get(4)?,
        capability_id: row.get(5)?,
    })
}

pub async fn list(conn: &Connection, todo_id: &str) -> Result<Vec<ProjectAssignment>> {
    let mut rows = conn.query(
        "SELECT todo_id,execution_id,kind,name,created_at,capability_id FROM project_todo_executions WHERE todo_id = ? ORDER BY created_at DESC, execution_id DESC",
        params![todo_id],
    ).await?;
    let mut assignments = Vec::new();
    while let Some(row) = rows.next().await? {
        assignments.push(assignment(&row)?);
    }
    Ok(assignments)
}

pub async fn latest(conn: &Connection) -> Result<Vec<ProjectAssignment>> {
    let mut rows = conn.query(
        "SELECT a.todo_id,a.execution_id,a.kind,a.name,a.created_at,a.capability_id FROM project_todo_executions a WHERE NOT EXISTS (SELECT 1 FROM project_todo_executions b WHERE b.todo_id = a.todo_id AND (b.created_at > a.created_at OR (b.created_at = a.created_at AND b.execution_id > a.execution_id)))",
        (),
    ).await?;
    let mut assignments = Vec::new();
    while let Some(row) = rows.next().await? {
        assignments.push(assignment(&row)?);
    }
    Ok(assignments)
}

pub async fn link(conn: &Connection, record: &ProjectAssignment) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO project_todo_executions (todo_id,execution_id,created_at,kind,name,capability_id) VALUES (?,?,?,?,?,?)",
        params![record.todo_id.as_str(), record.execution_id.as_str(), record.created_at, record.kind.as_str(), record.name.as_str(), record.capability_id.as_deref()],
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
