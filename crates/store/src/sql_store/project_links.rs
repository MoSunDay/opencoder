use crate::project::ProjectAssignment;
use anyhow::Result;
use sqlx::{MySqlPool, Row};

use super::{exec_read_all, exec_write, Arg};

fn assignment(row: &sqlx::mysql::MySqlRow) -> Result<ProjectAssignment> {
    Ok(ProjectAssignment {
        todo_id: row.try_get("todo_id")?,
        execution_id: row.try_get("execution_id")?,
        kind: row.try_get("kind")?,
        name: row.try_get("name")?,
        created_at: row.try_get("created_at")?,
        capability_id: row.try_get("capability_id")?,
    })
}

pub async fn list(
    pool: &MySqlPool,
    starrocks: bool,
    todo_id: &str,
) -> Result<Vec<ProjectAssignment>> {
    exec_read_all(pool, starrocks,
        "SELECT todo_id,execution_id,kind,name,created_at,capability_id FROM project_todo_executions WHERE todo_id = ? ORDER BY created_at DESC, execution_id DESC",
        &[Arg::Text(todo_id.to_owned())],
    ).await?.iter().map(assignment).collect()
}

pub async fn latest(pool: &MySqlPool, starrocks: bool) -> Result<Vec<ProjectAssignment>> {
    exec_read_all(pool, starrocks,
        "SELECT a.todo_id,a.execution_id,a.kind,a.name,a.created_at,a.capability_id FROM project_todo_executions a WHERE NOT EXISTS (SELECT 1 FROM project_todo_executions b WHERE b.todo_id = a.todo_id AND (b.created_at > a.created_at OR (b.created_at = a.created_at AND b.execution_id > a.execution_id)))", &[],
    ).await?.iter().map(assignment).collect()
}

pub async fn link(pool: &MySqlPool, starrocks: bool, record: &ProjectAssignment) -> Result<()> {
    if starrocks
        && !exec_read_all(
            pool,
            starrocks,
            "SELECT id FROM project_todo_executions WHERE id = ?",
            &[Arg::Text(format!(
                "{}:{}",
                record.todo_id, record.execution_id
            ))],
        )
        .await?
        .is_empty()
    {
        return Ok(());
    }
    let sql = if starrocks {
        "INSERT INTO project_todo_executions (id,todo_id,execution_id,created_at,kind,name,capability_id) VALUES (?,?,?,?,?,?,?)"
    } else {
        "INSERT IGNORE INTO project_todo_executions (id,todo_id,execution_id,created_at,kind,name,capability_id) VALUES (?,?,?,?,?,?,?)"
    };
    exec_write(
        pool,
        starrocks,
        sql,
        vec![
            Arg::Text(format!("{}:{}", record.todo_id, record.execution_id)),
            Arg::Text(record.todo_id.clone()),
            Arg::Text(record.execution_id.clone()),
            Arg::Int(record.created_at),
            Arg::Text(record.kind.clone()),
            Arg::Text(record.name.clone()),
            Arg::TextOrNull(record.capability_id.clone()),
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
