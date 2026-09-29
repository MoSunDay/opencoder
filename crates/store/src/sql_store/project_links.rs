use crate::project::{ProjectAssignment, ProjectAssignmentState};
use anyhow::{ensure, Result};
use sqlx::{MySqlPool, Row};

use super::{exec_read_all, exec_write, Arg};

fn assignment(row: &sqlx::mysql::MySqlRow) -> Result<ProjectAssignment> {
    Ok(ProjectAssignment {
        todo_id: row.try_get("todo_id")?,
        execution_id: row.try_get("execution_id")?,
        kind: row.try_get("kind")?,
        name: row.try_get("name")?,
        created_at: row.try_get("created_at")?,
        result_md: row.try_get("result_md")?,
        sync_state: row.try_get("sync_state")?,
    })
}

pub async fn list(
    pool: &MySqlPool,
    starrocks: bool,
    todo_id: &str,
) -> Result<Vec<ProjectAssignment>> {
    exec_read_all(pool, starrocks,
        "SELECT todo_id,execution_id,kind,name,created_at,result_md,sync_state FROM project_todo_executions WHERE todo_id = ? ORDER BY created_at DESC, execution_id DESC",
        &[Arg::Text(todo_id.to_owned())],
    ).await?.iter().map(assignment).collect()
}

pub async fn latest(pool: &MySqlPool, starrocks: bool) -> Result<Vec<ProjectAssignmentState>> {
    let rows = exec_read_all(pool, starrocks,
        "SELECT a.todo_id,a.execution_id,CASE WHEN a.result_md IS NOT NULL AND a.result_md <> '' THEN 'yes' ELSE 'no' END AS has_result,a.sync_state FROM project_todo_executions a WHERE NOT EXISTS (SELECT 1 FROM project_todo_executions b WHERE b.todo_id = a.todo_id AND (b.created_at > a.created_at OR (b.created_at = a.created_at AND b.execution_id > a.execution_id)))",
        &[],
    ).await?;
    rows.iter()
        .map(|row| {
            Ok(ProjectAssignmentState {
                todo_id: row.try_get("todo_id")?,
                execution_id: row.try_get("execution_id")?,
                has_result: row.try_get::<String, _>("has_result")? == "yes",
                sync_state: row.try_get("sync_state")?,
            })
        })
        .collect()
}

pub async fn pending(
    pool: &MySqlPool,
    starrocks: bool,
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
    exec_read_all(pool, starrocks,
        "SELECT todo_id,execution_id,kind,name,created_at,result_md,sync_state FROM project_todo_executions WHERE sync_state = 'pending' AND (todo_id > ? OR (todo_id = ? AND execution_id > ?)) ORDER BY todo_id, execution_id LIMIT ?",
        &[Arg::Text(after_todo.to_owned()), Arg::Text(after_todo.to_owned()), Arg::Text(after_execution.to_owned()), Arg::Int(limit as i64)],
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
        "INSERT INTO project_todo_executions (id,todo_id,execution_id,created_at,kind,name,result_md,sync_state) VALUES (?,?,?,?,?,?,?,?)"
    } else {
        "INSERT IGNORE INTO project_todo_executions (id,todo_id,execution_id,created_at,kind,name,result_md,sync_state) VALUES (?,?,?,?,?,?,?,?)"
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
            Arg::TextOrNull(record.result_md.clone()),
            Arg::Text(record.sync_state.clone()),
        ],
    )
    .await?;
    Ok(())
}

pub async fn finish(
    pool: &MySqlPool,
    starrocks: bool,
    todo_id: &str,
    execution_id: &str,
    state: &str,
    result_md: Option<&str>,
) -> Result<()> {
    exec_write(pool, starrocks,
        "UPDATE project_todo_executions SET sync_state = ?, result_md = ? WHERE todo_id = ? AND execution_id = ? AND sync_state = 'pending'",
        vec![Arg::Text(state.to_owned()), Arg::TextOrNull(result_md.map(str::to_owned)), Arg::Text(todo_id.to_owned()), Arg::Text(execution_id.to_owned())],
    ).await?;
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
