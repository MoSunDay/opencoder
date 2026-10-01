//! Resumable migration to the single initiative hierarchy and tag tables.
use super::super::{exec_read_all, exec_write, Arg};
use anyhow::{ensure, Result};
use sqlx::{MySqlPool, Row};

async fn columns(pool: &MySqlPool, sr: bool, table: &str) -> Result<Vec<String>> {
    exec_read_all(pool, sr, "SELECT column_name FROM information_schema.columns WHERE table_schema=DATABASE() AND table_name=?", &[Arg::Text(table.into())]).await?.iter().map(|r| Ok(r.try_get::<String, _>(0)?.to_lowercase())).collect()
}
async fn ddl(pool: &MySqlPool, sr: bool, sql: &str) -> Result<()> {
    if sr {
        sqlx::raw_sql(sql).execute(pool).await?;
    } else {
        sqlx::query(sql).execute(pool).await?;
    }
    Ok(())
}
async fn await_column(pool: &MySqlPool, sr: bool, name: &str, present: bool) -> Result<()> {
    for _ in 0..600 {
        if columns(pool, sr, "project_todos")
            .await?
            .iter()
            .any(|c| c == name)
            == present
        {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    anyhow::bail!("project catalog schema change has not completed")
}

pub(super) async fn initialize(pool: &MySqlPool, sr: bool) -> Result<()> {
    let tags = if sr {
        "CREATE TABLE IF NOT EXISTS project_tags (id VARCHAR(64) NOT NULL,scope_type VARCHAR(32) NOT NULL,scope_id VARCHAR(64) NOT NULL,name VARCHAR(128) NOT NULL) PRIMARY KEY(id) DISTRIBUTED BY HASH(id) BUCKETS 1"
    } else {
        "CREATE TABLE IF NOT EXISTS project_tags (id VARCHAR(64) PRIMARY KEY,scope_type VARCHAR(32) NOT NULL,scope_id VARCHAR(64) NOT NULL,name VARCHAR(128) COLLATE utf8mb4_bin NOT NULL,UNIQUE KEY tag_scope_name(scope_type,scope_id,name)) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4"
    };
    let links = if sr {
        "CREATE TABLE IF NOT EXISTS project_todo_tags(todo_id VARCHAR(64) NOT NULL,tag_id VARCHAR(64) NOT NULL) PRIMARY KEY(todo_id,tag_id) DISTRIBUTED BY HASH(todo_id) BUCKETS 1"
    } else {
        "CREATE TABLE IF NOT EXISTS project_todo_tags(todo_id VARCHAR(64) NOT NULL,tag_id VARCHAR(64) NOT NULL,PRIMARY KEY(todo_id,tag_id)) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4"
    };
    ddl(pool, sr, tags).await?;
    ddl(pool, sr, links).await
}

pub(super) async fn upgrade(pool: &MySqlPool, sr: bool) -> Result<()> {
    let old = columns(pool, sr, "project_milestones").await?;
    let todo_columns = columns(pool, sr, "project_todos").await?;
    if !old.is_empty() && old.iter().any(|c| c == "kind") {
        exec_write(pool, sr, "INSERT INTO project_initiatives (id,goal_id,title,detail_md,status,sort_key,created_at,updated_at) SELECT id,goal_id,title,detail_md,status,sort_key,created_at,updated_at FROM project_milestones WHERE kind='initiative' AND id NOT IN (SELECT id FROM project_initiatives)", vec![]).await?;
        let rows = exec_read_all(pool, sr, "SELECT COUNT(*) FROM project_milestones m LEFT JOIN project_initiatives i ON m.id=i.id WHERE m.kind='initiative' AND (i.id IS NULL OR NOT(m.title <=> i.title) OR NOT(m.goal_id <=> i.goal_id) OR NOT(m.detail_md <=> i.detail_md) OR NOT(m.status <=> i.status) OR NOT(m.sort_key <=> i.sort_key) OR NOT(m.created_at <=> i.created_at) OR NOT(m.updated_at <=> i.updated_at))", &[]).await?;
        ensure!(
            rows[0].try_get::<i64, _>(0)? == 0,
            "initiative migration would lose data"
        );
    }
    if todo_columns.iter().any(|c| c == "milestone_id") {
        if !todo_columns.iter().any(|c| c == "initiative_id") {
            ddl(
                pool,
                sr,
                "ALTER TABLE project_todos ADD COLUMN initiative_id VARCHAR(64) NULL",
            )
            .await?;
            await_column(pool, sr, "initiative_id", true).await?;
        }
        exec_write(pool, sr, "UPDATE project_todos SET initiative_id=milestone_id WHERE milestone_id IN (SELECT id FROM project_initiatives)", vec![]).await?;
        ddl(
            pool,
            sr,
            "ALTER TABLE project_todos DROP COLUMN milestone_id",
        )
        .await?;
        await_column(pool, sr, "milestone_id", false).await?;
    }
    if !old.is_empty() {
        ddl(pool, sr, "DROP TABLE project_milestones").await?;
    }
    initialize(pool, sr).await
}
