use anyhow::Result;
use sqlx::{MySqlPool, Row};

use super::{exec_read_all, exec_write, Arg};

pub async fn list(pool: &MySqlPool, starrocks: bool, todo_id: &str) -> Result<Vec<String>> {
    let rows = exec_read_all(
        pool,
        starrocks,
        "SELECT execution_id FROM project_todo_executions WHERE todo_id = ? ORDER BY created_at DESC, execution_id",
        &[Arg::Text(todo_id.to_owned())],
    )
    .await?;
    rows.iter()
        .map(|row| Ok(row.try_get("execution_id")?))
        .collect()
}

pub async fn link(
    pool: &MySqlPool,
    starrocks: bool,
    todo_id: &str,
    execution_id: &str,
) -> Result<()> {
    let id = format!("{todo_id}:{execution_id}");
    let sql = if starrocks {
        "INSERT INTO project_todo_executions (id, todo_id, execution_id, created_at) VALUES (?, ?, ?, ?)"
    } else {
        "INSERT IGNORE INTO project_todo_executions (id, todo_id, execution_id, created_at) VALUES (?, ?, ?, ?)"
    };
    exec_write(
        pool,
        starrocks,
        sql,
        vec![
            Arg::Text(id),
            Arg::Text(todo_id.to_owned()),
            Arg::Text(execution_id.to_owned()),
            Arg::Int(opencoder_core::message::now_ms()),
        ],
    )
    .await?;
    Ok(())
}

pub async fn unlink(
    pool: &MySqlPool,
    starrocks: bool,
    todo_id: &str,
    execution_id: &str,
) -> Result<bool> {
    Ok(exec_write(
        pool,
        starrocks,
        "DELETE FROM project_todo_executions WHERE todo_id = ? AND execution_id = ?",
        vec![
            Arg::Text(todo_id.to_owned()),
            Arg::Text(execution_id.to_owned()),
        ],
    )
    .await?
        > 0)
}
