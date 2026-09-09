//! Nullability + resumable one-time backlog migration. Column metadata is the
//! migration checkpoint, so normal future backlog items remain independent.
use super::super::{exec_read_all, exec_write, Arg};
use anyhow::{ensure, Context, Result};
use sqlx::{MySqlPool, Row};

const PENDING: &str = "opencoder:relations-v23:pending";
const COMPLETE: &str = "opencoder:relations-v23";
const BACKLOG: &str = "pm-backlog-v23";

async fn column(pool: &MySqlPool, starrocks: bool) -> Result<(bool, String)> {
    let rows = exec_read_all(pool, starrocks,
        "SELECT IS_NULLABLE,COLUMN_COMMENT FROM information_schema.columns WHERE table_schema = DATABASE() AND table_name = 'project_milestones' AND column_name = 'goal_id'", &[]).await?;
    let row = rows
        .first()
        .context("project_milestones.goal_id metadata missing")?;
    Ok((
        row.try_get::<String, _>(0)?.eq_ignore_ascii_case("YES"),
        row.try_get(1)?,
    ))
}

async fn mark(pool: &MySqlPool, starrocks: bool, comment: &str) -> Result<()> {
    // comment is one of the private constants above, never caller input.
    let sql = format!(
        "ALTER TABLE project_milestones MODIFY COLUMN goal_id VARCHAR(64) NULL COMMENT '{comment}'"
    );
    exec_write(pool, starrocks, &sql, vec![]).await?;
    // StarRocks schema changes publish asynchronously; do not expose a store
    // until the requested schema is visible. A timeout leaves a retryable marker.
    for _ in 0..600 {
        if column(pool, starrocks).await? == (true, comment.to_owned()) {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    anyhow::bail!("project relation schema change has not completed")
}

pub(super) async fn upgrade(pool: &MySqlPool, starrocks: bool) -> Result<()> {
    let (nullable, comment) = column(pool, starrocks).await?;
    if nullable && comment != PENDING {
        return Ok(());
    }
    if !nullable {
        mark(pool, starrocks, PENDING).await?;
    }
    let todos = exec_read_all(
        pool,
        starrocks,
        "SELECT id FROM project_todos WHERE milestone_id IS NULL",
        &[],
    )
    .await?;
    if !todos.is_empty() {
        let existing = exec_read_all(
            pool,
            starrocks,
            "SELECT title,goal_id FROM project_milestones WHERE id = ?",
            &[Arg::Text(BACKLOG.into())],
        )
        .await?;
        if let Some(row) = existing.first() {
            ensure!(
                row.try_get::<String, _>(0)? == "待归类"
                    && row.try_get::<Option<String>, _>(1)?.is_none(),
                "legacy backlog milestone id collision"
            );
        } else {
            let now = opencoder_core::message::now_ms();
            exec_write(pool, starrocks,
                "INSERT INTO project_milestones (id,goal_id,title,detail_md,status,sort_key,created_at,updated_at) VALUES (?,NULL,'待归类',NULL,'planned',0,?,?)",
                vec![Arg::Text(BACKLOG.into()), Arg::Int(now), Arg::Int(now)]).await?;
        }
        exec_write(
            pool,
            starrocks,
            "UPDATE project_todos SET milestone_id = ? WHERE milestone_id IS NULL",
            vec![Arg::Text(BACKLOG.into())],
        )
        .await?;
        let remaining = exec_read_all(
            pool,
            starrocks,
            "SELECT id FROM project_todos WHERE milestone_id IS NULL LIMIT 1",
            &[],
        )
        .await?;
        ensure!(
            remaining.is_empty(),
            "legacy backlog classification has not completed"
        );
    }
    mark(pool, starrocks, COMPLETE).await
}
