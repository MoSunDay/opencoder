//! Project-module persistence — goals & initiatives CRUD (MySQL dialect).
//!
//! Companion of [`super::project_crud_runs`] (todos & runs). Free functions
//! over a `MySqlPool`; deletes cascade explicitly and run through
//! [`super::run_cascade`] (a real transaction on MySQL, sequential
//! statements on StarRocks). Behavior mirrors `libsql_store::project`.

use anyhow::{Context, Result};
use sqlx::{MySqlPool, Row};

use super::{corrupt_status, exec_read_all, exec_write, Arg};
use crate::project_types::{
    ProjectGoalPatch, ProjectGoalRecord, ProjectGoalStatus, ProjectInitiativeRecord,
    ProjectInitiativeStatus,
};

const GOAL_COLS: &str = "id, title, detail_md, status, sort_key, created_at, updated_at";
const INITIATIVE_COLS: &str =
    "id, goal_id, title, detail_md, status, sort_key, created_at, updated_at";

// ---- goals ----

pub async fn create_goal(pool: &MySqlPool, starrocks: bool, rec: &ProjectGoalRecord) -> Result<()> {
    exec_write(
        pool,
        starrocks,
        "INSERT INTO project_goals \
         (id, title, detail_md, status, sort_key, created_at, updated_at) \
         VALUES (?,?,?,?,?,?,?)",
        vec![
            Arg::Text(rec.id.clone()),
            Arg::Text(rec.title.clone()),
            Arg::TextOrNull(rec.detail_md.clone()),
            Arg::Text(rec.status.as_str().to_string()),
            Arg::Int(rec.sort),
            Arg::Int(rec.created_at),
            Arg::Int(rec.updated_at),
        ],
    )
    .await
    .context("insert project goal")?;
    Ok(())
}

/// Dynamic `SET` from the patch's `Some` fields; always stamps
/// `updated_at = now_ms`. Returns `false` when the id does not exist (0
/// matched rows — sqlx negotiates CLIENT_FOUND_ROWS, so matched == affected
/// and a no-op patch of a live row still reports `true`, like libsql).
pub async fn patch_goal(
    pool: &MySqlPool,
    starrocks: bool,
    id: &str,
    patch: &ProjectGoalPatch,
    now_ms: i64,
) -> Result<bool> {
    let mut sets: Vec<&'static str> = Vec::new();
    let mut args: Vec<Arg> = Vec::new();
    if let Some(v) = &patch.title {
        sets.push("title = ?");
        args.push(Arg::Text(v.clone()));
    }
    if let Some(v) = &patch.detail_md {
        sets.push("detail_md = ?");
        args.push(Arg::Text(v.clone()));
    }
    if let Some(v) = patch.status {
        sets.push("status = ?");
        args.push(Arg::Text(v.as_str().to_string()));
    }
    if let Some(v) = patch.sort {
        sets.push("sort_key = ?");
        args.push(Arg::Int(v));
    }
    sets.push("updated_at = ?");
    args.push(Arg::Int(now_ms));
    args.push(Arg::Text(id.to_string()));
    let sql = format!("UPDATE project_goals SET {} WHERE id = ?", sets.join(", "));
    let n = exec_write(pool, starrocks, &sql, args)
        .await
        .context("patch project goal")?;
    Ok(n > 0)
}

/// Ordered by `sort_key` then `created_at`.
pub async fn list_goals(pool: &MySqlPool, starrocks: bool) -> Result<Vec<ProjectGoalRecord>> {
    let rows = exec_read_all(
        pool,
        starrocks,
        &format!("SELECT {GOAL_COLS} FROM project_goals ORDER BY sort_key, created_at"),
        &[],
    )
    .await?;
    rows.iter().map(row_to_goal).collect()
}

fn row_to_goal(r: &sqlx::mysql::MySqlRow) -> Result<ProjectGoalRecord> {
    let status: String = r.try_get("status")?;
    Ok(ProjectGoalRecord {
        id: r.try_get("id")?,
        title: r.try_get("title")?,
        detail_md: r.try_get::<Option<String>, _>("detail_md")?,
        status: ProjectGoalStatus::parse(&status)
            .ok_or_else(|| corrupt_status("project_goals.status", &status))?,
        sort: r.try_get("sort_key")?,
        created_at: r.try_get("created_at")?,
        updated_at: r.try_get("updated_at")?,
    })
}

// ---- initiatives ----

pub async fn create_initiative(
    pool: &MySqlPool,
    starrocks: bool,
    rec: &ProjectInitiativeRecord,
) -> Result<()> {
    exec_write(
        pool,
        starrocks,
        "INSERT INTO project_initiatives \
         (id, goal_id, title, detail_md, status, sort_key, created_at, updated_at) \
         VALUES (?,?,?,?,?,?,?,?)",
        vec![
            Arg::Text(rec.id.clone()),
            Arg::TextOrNull(rec.goal_id.clone()),
            Arg::Text(rec.title.clone()),
            Arg::TextOrNull(rec.detail_md.clone()),
            Arg::Text(rec.status.as_str().to_string()),
            Arg::Int(rec.sort),
            Arg::Int(rec.created_at),
            Arg::Int(rec.updated_at),
        ],
    )
    .await
    .context("insert project initiative")?;
    Ok(())
}

/// `goal_id == None` lists across all goals; ordered by `sort_key` then
/// `created_at`.
pub async fn list_initiatives(
    pool: &MySqlPool,
    starrocks: bool,
    goal_id: Option<&str>,
) -> Result<Vec<ProjectInitiativeRecord>> {
    let mut sql = format!("SELECT {INITIATIVE_COLS} FROM project_initiatives ");
    let mut args: Vec<Arg> = vec![];
    if let Some(g) = goal_id {
        sql.push_str(" WHERE goal_id = ?");
        args.push(Arg::Text(g.to_string()));
    }
    sql.push_str(" ORDER BY sort_key, created_at");
    let rows = exec_read_all(pool, starrocks, &sql, &args).await?;
    rows.iter().map(row_to_initiative).collect()
}

pub(super) fn row_to_initiative(r: &sqlx::mysql::MySqlRow) -> Result<ProjectInitiativeRecord> {
    let status: String = r.try_get("status")?;
    Ok(ProjectInitiativeRecord {
        id: r.try_get("id")?,
        goal_id: r.try_get("goal_id")?,
        title: r.try_get("title")?,
        detail_md: r.try_get::<Option<String>, _>("detail_md")?,
        status: ProjectInitiativeStatus::parse(&status)
            .ok_or_else(|| corrupt_status("project_initiatives.status", &status))?,
        sort: r.try_get("sort_key")?,
        created_at: r.try_get("created_at")?,
        updated_at: r.try_get("updated_at")?,
    })
}
