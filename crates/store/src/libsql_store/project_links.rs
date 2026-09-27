use anyhow::Result;
use libsql::{params, Connection};

pub async fn list(conn: &Connection, todo_id: &str) -> Result<Vec<String>> {
    let mut rows = conn
        .query(
            "SELECT execution_id FROM project_todo_executions WHERE todo_id = ? ORDER BY created_at DESC, execution_id",
            params![todo_id],
        )
        .await?;
    let mut ids = Vec::new();
    while let Some(row) = rows.next().await? {
        ids.push(row.get(0)?);
    }
    Ok(ids)
}

pub async fn link(conn: &Connection, todo_id: &str, execution_id: &str) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO project_todo_executions (todo_id, execution_id, created_at) VALUES (?, ?, ?)",
        params![todo_id, execution_id, opencoder_core::message::now_ms()],
    )
    .await?;
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
