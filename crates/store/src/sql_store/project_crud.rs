//! Project-module persistence — goals & milestones CRUD (MySQL dialect).
//!
//! Companion of [`super::project_crud_runs`] (todos & runs). Free functions
//! over a `MySqlPool`; deletes cascade explicitly and run through
//! [`super::run_cascade`] (a real transaction on MySQL, sequential
//! statements on StarRocks). Behavior mirrors `libsql_store::project`.

use anyhow::{Context, Result};
use sqlx::{MySqlPool, Row};

use super::{corrupt_status, exec_read_all, exec_write, id_column, row_exists, run_cascade, Arg};
use crate::project_types::{
    ProjectGoalPatch, ProjectGoalRecord, ProjectGoalStatus, ProjectMilestonePatch,
    ProjectMilestoneRecord, ProjectMilestoneStatus,
};

const GOAL_COLS: &str = "id, title, detail_md, status, sort_key, created_at, updated_at";
const MILESTONE_COLS: &str =
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

/// Cascade: the runs of the goal's todos → the todos → the milestones →
/// the goal. `false` when the goal id does not exist. The todo ids are
/// SELECTed up front so every DELETE binds a flat single id (portable to
/// StarRocks, whose DELETE grammar is stricter about subqueries).
pub async fn delete_goal(pool: &MySqlPool, starrocks: bool, id: &str) -> Result<bool> {
    if !row_exists(
        pool,
        starrocks,
        "SELECT 1 FROM project_goals WHERE id = ?",
        id,
    )
    .await?
    {
        return Ok(false);
    }
    let todo_rows = exec_read_all(
        pool,
        starrocks,
        "SELECT t.id FROM project_todos t \
         JOIN project_milestones m ON m.id = t.milestone_id WHERE m.goal_id = ?",
        &[Arg::Text(id.to_string())],
    )
    .await?;
    let todo_ids = id_column(&todo_rows, "id")?;
    let mut stmts: Vec<(&'static str, String)> = Vec::new();
    for tid in &todo_ids {
        stmts.push((
            "DELETE FROM project_todo_runs WHERE todo_id = ?",
            tid.clone(),
        ));
    }
    for tid in &todo_ids {
        stmts.push(("DELETE FROM project_todos WHERE id = ?", tid.clone()));
    }
    stmts.push((
        "DELETE FROM project_milestones WHERE goal_id = ?",
        id.to_string(),
    ));
    stmts.push(("DELETE FROM project_goals WHERE id = ?", id.to_string()));
    run_cascade(pool, starrocks, &stmts).await?;
    Ok(true)
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

// ---- milestones ----

pub async fn create_milestone(
    pool: &MySqlPool,
    starrocks: bool,
    rec: &ProjectMilestoneRecord,
) -> Result<()> {
    exec_write(
        pool,
        starrocks,
        "INSERT INTO project_milestones \
         (id, goal_id, title, detail_md, status, sort_key, created_at, updated_at) \
         VALUES (?,?,?,?,?,?,?,?)",
        vec![
            Arg::Text(rec.id.clone()),
            Arg::Text(rec.goal_id.clone()),
            Arg::Text(rec.title.clone()),
            Arg::TextOrNull(rec.detail_md.clone()),
            Arg::Text(rec.status.as_str().to_string()),
            Arg::Int(rec.sort),
            Arg::Int(rec.created_at),
            Arg::Int(rec.updated_at),
        ],
    )
    .await
    .context("insert project milestone")?;
    Ok(())
}

pub async fn patch_milestone(
    pool: &MySqlPool,
    starrocks: bool,
    id: &str,
    patch: &ProjectMilestonePatch,
    now_ms: i64,
) -> Result<bool> {
    let mut sets: Vec<&'static str> = Vec::new();
    let mut args: Vec<Arg> = Vec::new();
    if let Some(v) = &patch.goal_id {
        sets.push("goal_id = ?");
        args.push(Arg::Text(v.clone()));
    }
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
    let sql = format!(
        "UPDATE project_milestones SET {} WHERE id = ?",
        sets.join(", ")
    );
    let n = exec_write(pool, starrocks, &sql, args)
        .await
        .context("patch project milestone")?;
    Ok(n > 0)
}

/// Cascade: the runs of this milestone's todos → the todos themselves →
/// the milestone. Todos are deleted, NOT re-parented to the backlog (see the
/// trait docs). `false` when the milestone id does not exist.
pub async fn delete_milestone(pool: &MySqlPool, starrocks: bool, id: &str) -> Result<bool> {
    if !row_exists(
        pool,
        starrocks,
        "SELECT 1 FROM project_milestones WHERE id = ?",
        id,
    )
    .await?
    {
        return Ok(false);
    }
    let todo_rows = exec_read_all(
        pool,
        starrocks,
        "SELECT id FROM project_todos WHERE milestone_id = ?",
        &[Arg::Text(id.to_string())],
    )
    .await?;
    let todo_ids = id_column(&todo_rows, "id")?;
    let mut stmts: Vec<(&'static str, String)> = Vec::new();
    for tid in &todo_ids {
        stmts.push((
            "DELETE FROM project_todo_runs WHERE todo_id = ?",
            tid.clone(),
        ));
    }
    stmts.push((
        "DELETE FROM project_todos WHERE milestone_id = ?",
        id.to_string(),
    ));
    stmts.push((
        "DELETE FROM project_milestones WHERE id = ?",
        id.to_string(),
    ));
    run_cascade(pool, starrocks, &stmts).await?;
    Ok(true)
}

/// `goal_id == None` lists across all goals; ordered by `sort_key` then
/// `created_at`.
pub async fn list_milestones(
    pool: &MySqlPool,
    starrocks: bool,
    goal_id: Option<&str>,
) -> Result<Vec<ProjectMilestoneRecord>> {
    let mut sql = format!("SELECT {MILESTONE_COLS} FROM project_milestones");
    let mut args: Vec<Arg> = Vec::new();
    if let Some(g) = goal_id {
        sql.push_str(" WHERE goal_id = ?");
        args.push(Arg::Text(g.to_string()));
    }
    sql.push_str(" ORDER BY sort_key, created_at");
    let rows = exec_read_all(pool, starrocks, &sql, &args).await?;
    rows.iter().map(row_to_milestone).collect()
}

fn row_to_milestone(r: &sqlx::mysql::MySqlRow) -> Result<ProjectMilestoneRecord> {
    let status: String = r.try_get("status")?;
    Ok(ProjectMilestoneRecord {
        id: r.try_get("id")?,
        goal_id: r.try_get("goal_id")?,
        title: r.try_get("title")?,
        detail_md: r.try_get::<Option<String>, _>("detail_md")?,
        status: ProjectMilestoneStatus::parse(&status)
            .ok_or_else(|| corrupt_status("project_milestones.status", &status))?,
        sort: r.try_get("sort_key")?,
        created_at: r.try_get("created_at")?,
        updated_at: r.try_get("updated_at")?,
    })
}
