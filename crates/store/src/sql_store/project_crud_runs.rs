//! Project-module persistence — todos & todo runs CRUD (MySQL dialect).
//!
//! Companion of [`super::project_crud`] (goals & milestones). Behavior
//! mirrors `libsql_store::project_runs`: dynamic SET patches with
//! `Option<Option<String>>` clearing, newest-first run listing,
//! `COALESCE(MAX(version), 0) + 1` versioning.

use anyhow::{Context, Result};
use sqlx::{MySqlPool, Row};

use super::{
    corrupt_status, exec_read_all, exec_read_opt, exec_write, row_exists, run_cascade, Arg,
};
use crate::project_types::{
    ProjectTodoRecord, ProjectTodoRunKind, ProjectTodoRunPatch, ProjectTodoRunRecord,
    ProjectTodoRunStatus, ProjectTodoStatus,
};

const TODO_COLS: &str = "id, milestone_id, title, draft, plan_md, status, agent, active_session_id, created_at, updated_at";
const RUN_COLS: &str = "id, todo_id, kind, version, plan_md, output_md, agent, session_id, status, started_at, finished_at, created_at";

// ---- todos ----

pub async fn create_todo(pool: &MySqlPool, starrocks: bool, rec: &ProjectTodoRecord) -> Result<()> {
    exec_write(
        pool,
        starrocks,
        "INSERT INTO project_todos \
         (id, milestone_id, title, draft, plan_md, status, agent, active_session_id, created_at, updated_at) \
         VALUES (?,?,?,?,?,?,?,?,?,?)",
        vec![
            Arg::Text(rec.id.clone()),
            Arg::TextOrNull(rec.milestone_id.clone()),
            Arg::Text(rec.title.clone()),
            Arg::Text(rec.draft.clone()),
            Arg::TextOrNull(rec.plan_md.clone()),
            Arg::Text(rec.status.as_str().to_string()),
            Arg::Text(rec.agent.clone()),
            Arg::TextOrNull(rec.active_session_id.clone()),
            Arg::Int(rec.created_at),
            Arg::Int(rec.updated_at),
        ],
    )
    .await
    .context("insert project todo")?;
    Ok(())
}

/// Dynamic `SET` from the patch's `Some` fields; always stamps
/// `updated_at = now_ms`. `Option<Option<String>>` fields distinguish "leave
/// unchanged" (outer `None`) from "clear to NULL" (`Some(None)`). Returns
/// `false` when the id does not exist.
pub async fn patch_todo(
    pool: &MySqlPool,
    starrocks: bool,
    id: &str,
    patch: &crate::project_types::ProjectTodoPatch,
    now_ms: i64,
) -> Result<bool> {
    let mut sets: Vec<&'static str> = Vec::new();
    let mut args: Vec<Arg> = Vec::new();
    if let Some(v) = &patch.title {
        sets.push("title = ?");
        args.push(Arg::Text(v.clone()));
    }
    if let Some(v) = &patch.draft {
        sets.push("draft = ?");
        args.push(Arg::Text(v.clone()));
    }
    if let Some(v) = patch.plan_md.as_ref() {
        sets.push("plan_md = ?");
        args.push(Arg::TextOrNull(v.clone())); // Some(None) -> NULL
    }
    if let Some(v) = patch.status {
        sets.push("status = ?");
        args.push(Arg::Text(v.as_str().to_string()));
    }
    if let Some(v) = &patch.agent {
        sets.push("agent = ?");
        args.push(Arg::Text(v.clone()));
    }
    if let Some(v) = patch.milestone_id.as_ref() {
        sets.push("milestone_id = ?");
        args.push(Arg::TextOrNull(v.clone()));
    }
    if let Some(v) = patch.active_session_id.as_ref() {
        sets.push("active_session_id = ?");
        args.push(Arg::TextOrNull(v.clone()));
    }
    sets.push("updated_at = ?");
    args.push(Arg::Int(now_ms));
    args.push(Arg::Text(id.to_string()));
    let sql = format!("UPDATE project_todos SET {} WHERE id = ?", sets.join(", "));
    let n = exec_write(pool, starrocks, &sql, args)
        .await
        .context("patch project todo")?;
    Ok(n > 0)
}

/// Cascade: the todo's runs, then the todo. `false` when the id does not
/// exist.
pub async fn delete_todo(pool: &MySqlPool, starrocks: bool, id: &str) -> Result<bool> {
    if !row_exists(
        pool,
        starrocks,
        "SELECT 1 FROM project_todos WHERE id = ?",
        id,
    )
    .await?
    {
        return Ok(false);
    }
    let stmts = [
        (
            "DELETE FROM project_todo_runs WHERE todo_id = ?",
            id.to_string(),
        ),
        ("DELETE FROM project_todos WHERE id = ?", id.to_string()),
    ];
    run_cascade(pool, starrocks, &stmts).await?;
    Ok(true)
}

pub async fn get_todo(
    pool: &MySqlPool,
    starrocks: bool,
    id: &str,
) -> Result<Option<ProjectTodoRecord>> {
    let row = exec_read_opt(
        pool,
        starrocks,
        &format!("SELECT {TODO_COLS} FROM project_todos WHERE id = ? LIMIT 1"),
        &[Arg::Text(id.to_string())],
    )
    .await?;
    row.as_ref().map(row_to_todo).transpose()
}

/// `milestone_id == None` lists ALL todos (backlog included); ordered by
/// `created_at`.
pub async fn list_todos(
    pool: &MySqlPool,
    starrocks: bool,
    milestone_id: Option<&str>,
) -> Result<Vec<ProjectTodoRecord>> {
    let mut sql = format!("SELECT {TODO_COLS} FROM project_todos");
    let mut args: Vec<Arg> = Vec::new();
    if let Some(m) = milestone_id {
        sql.push_str(" WHERE milestone_id = ?");
        args.push(Arg::Text(m.to_string()));
    }
    sql.push_str(" ORDER BY created_at");
    let rows = exec_read_all(pool, starrocks, &sql, &args).await?;
    rows.iter().map(row_to_todo).collect()
}

fn row_to_todo(r: &sqlx::mysql::MySqlRow) -> Result<ProjectTodoRecord> {
    let status: String = r.try_get("status")?;
    Ok(ProjectTodoRecord {
        id: r.try_get("id")?,
        milestone_id: r.try_get::<Option<String>, _>("milestone_id")?,
        title: r.try_get("title")?,
        draft: r.try_get("draft")?,
        plan_md: r.try_get::<Option<String>, _>("plan_md")?,
        status: ProjectTodoStatus::parse(&status)
            .ok_or_else(|| corrupt_status("project_todos.status", &status))?,
        agent: r.try_get("agent")?,
        active_session_id: r.try_get::<Option<String>, _>("active_session_id")?,
        created_at: r.try_get("created_at")?,
        updated_at: r.try_get("updated_at")?,
    })
}

// ---- todo runs ----

pub async fn create_todo_run(
    pool: &MySqlPool,
    starrocks: bool,
    rec: &ProjectTodoRunRecord,
) -> Result<()> {
    exec_write(
        pool,
        starrocks,
        "INSERT INTO project_todo_runs \
         (id, todo_id, kind, version, plan_md, output_md, agent, session_id, status, started_at, finished_at, created_at) \
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
        vec![
            Arg::Text(rec.id.clone()),
            Arg::Text(rec.todo_id.clone()),
            Arg::Text(rec.kind.as_str().to_string()),
            Arg::Int(rec.version),
            Arg::TextOrNull(rec.plan_md.clone()),
            Arg::TextOrNull(rec.output_md.clone()),
            Arg::Text(rec.agent.clone()),
            Arg::TextOrNull(rec.session_id.clone()),
            Arg::Text(rec.status.as_str().to_string()),
            Arg::Int(rec.started_at),
            Arg::IntOrNull(rec.finished_at),
            Arg::Int(rec.created_at),
        ],
    )
    .await
    .context("insert project todo run")?;
    Ok(())
}

pub async fn patch_todo_run(
    pool: &MySqlPool,
    starrocks: bool,
    id: &str,
    patch: &ProjectTodoRunPatch,
    _now_ms: i64,
) -> Result<bool> {
    let mut sets: Vec<&'static str> = Vec::new();
    let mut args: Vec<Arg> = Vec::new();
    // Plain Option<String> fields: Some(v) sets, never clears.
    if let Some(v) = &patch.plan_md {
        sets.push("plan_md = ?");
        args.push(Arg::Text(v.clone()));
    }
    if let Some(v) = &patch.output_md {
        sets.push("output_md = ?");
        args.push(Arg::Text(v.clone()));
    }
    if let Some(v) = &patch.session_id {
        sets.push("session_id = ?");
        args.push(Arg::Text(v.clone()));
    }
    if let Some(v) = patch.status {
        sets.push("status = ?");
        args.push(Arg::Text(v.as_str().to_string()));
    }
    if let Some(v) = patch.finished_at {
        sets.push("finished_at = ?");
        args.push(Arg::Int(v));
    }
    // No updated_at column on runs (created_at + finished_at span the
    // lifecycle); now_ms stays unused for signature uniformity, same as the
    // libsql impl.
    args.push(Arg::Text(id.to_string()));
    let sql = format!(
        "UPDATE project_todo_runs SET {} WHERE id = ?",
        sets.join(", ")
    );
    let n = exec_write(pool, starrocks, &sql, args)
        .await
        .context("patch project todo run")?;
    Ok(n > 0)
}

pub async fn get_todo_run(
    pool: &MySqlPool,
    starrocks: bool,
    id: &str,
) -> Result<Option<ProjectTodoRunRecord>> {
    let row = exec_read_opt(
        pool,
        starrocks,
        &format!("SELECT {RUN_COLS} FROM project_todo_runs WHERE id = ? LIMIT 1"),
        &[Arg::Text(id.to_string())],
    )
    .await?;
    row.as_ref().map(row_to_run).transpose()
}

/// Newest version first.
pub async fn list_todo_runs(
    pool: &MySqlPool,
    starrocks: bool,
    todo_id: &str,
) -> Result<Vec<ProjectTodoRunRecord>> {
    let rows = exec_read_all(
        pool,
        starrocks,
        &format!(
            "SELECT {RUN_COLS} FROM project_todo_runs WHERE todo_id = ? ORDER BY version DESC"
        ),
        &[Arg::Text(todo_id.to_string())],
    )
    .await?;
    rows.iter().map(row_to_run).collect()
}

/// `COALESCE(MAX(version), 0) + 1` — 1 for a todo with no runs yet. The
/// alias keeps row access by name, consistent with every other read here.
pub async fn next_todo_version(pool: &MySqlPool, starrocks: bool, todo_id: &str) -> Result<i64> {
    let rows = exec_read_all(
        pool,
        starrocks,
        "SELECT COALESCE(MAX(version), 0) + 1 AS next_version \
         FROM project_todo_runs WHERE todo_id = ?",
        &[Arg::Text(todo_id.to_string())],
    )
    .await?;
    // Aggregate over the filtered set: exactly one row in practice; an empty
    // result (no GROUP BY aggregate row) would itself be corruption.
    let row = rows
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("corrupt project row: empty next_todo_version result"))?;
    Ok(row.try_get::<i64, _>("next_version")?)
}

fn row_to_run(r: &sqlx::mysql::MySqlRow) -> Result<ProjectTodoRunRecord> {
    let kind: String = r.try_get("kind")?;
    let status: String = r.try_get("status")?;
    Ok(ProjectTodoRunRecord {
        id: r.try_get("id")?,
        todo_id: r.try_get("todo_id")?,
        kind: ProjectTodoRunKind::parse(&kind)
            .ok_or_else(|| anyhow::anyhow!("corrupt project row: unknown kind {kind}"))?,
        version: r.try_get("version")?,
        plan_md: r.try_get::<Option<String>, _>("plan_md")?,
        output_md: r.try_get::<Option<String>, _>("output_md")?,
        agent: r.try_get("agent")?,
        session_id: r.try_get::<Option<String>, _>("session_id")?,
        status: ProjectTodoRunStatus::parse(&status)
            .ok_or_else(|| corrupt_status("project_todo_runs.status", &status))?,
        started_at: r.try_get("started_at")?,
        finished_at: r.try_get::<Option<i64>, _>("finished_at")?,
        created_at: r.try_get("created_at")?,
    })
}
