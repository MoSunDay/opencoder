//! Project-module persistence — todo CRUD (MySQL dialect).
//!
//! The todo half of the project tables; companion of
//! [`super::project_crud_runs`] (todo runs) and [`super::project_crud`]
//! (goals & milestones). Behavior mirrors `libsql_store::project_runs`:
//! dynamic SET patches with `Option<Option<String>>` clearing and
//! expected-status CAS claims.

use anyhow::{Context, Result};
use sqlx::{MySqlPool, Row};

use super::{
    corrupt_status, exec_read_all, exec_read_opt, exec_write, row_exists, run_cascade, Arg,
};
use crate::project_types::{ProjectExecutorKind, ProjectTodoRecord, ProjectTodoStatus};

const TODO_COLS: &str = "id, milestone_id, title, draft, plan_md, status, agent, active_session_id, created_at, updated_at, executor_kind, executor_ref, executor_spec";

// ---- todos ----

pub async fn create_todo(pool: &MySqlPool, starrocks: bool, rec: &ProjectTodoRecord) -> Result<()> {
    exec_write(
        pool,
        starrocks,
        "INSERT INTO project_todos \
         (id, milestone_id, title, draft, plan_md, status, agent, active_session_id, created_at, updated_at, executor_kind, executor_ref, executor_spec) \
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)",
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
            Arg::Text(rec.executor_kind.as_str().to_string()),
            Arg::TextOrNull(rec.executor_ref.clone()),
            Arg::TextOrNull(rec.executor_spec.clone()),
        ],
    )
    .await
    .context("insert project todo")?;
    Ok(())
}

/// The `SET` fragments + bound args shared by `patch_todo` and its
/// expected-status CAS variant — pure projection of the patch's `Some`
/// fields, no I/O. `Option<Option<String>>` fields distinguish "leave
/// unchanged" (outer `None`) from "clear to NULL" (`Some(None)`).
fn todo_set_fragment(
    patch: &crate::project_types::ProjectTodoPatch,
) -> (Vec<&'static str>, Vec<Arg>) {
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
    if let Some(v) = patch.executor_kind {
        sets.push("executor_kind = ?");
        args.push(Arg::Text(v.as_str().to_string()));
    }
    if let Some(v) = patch.executor_ref.as_ref() {
        sets.push("executor_ref = ?");
        args.push(Arg::TextOrNull(v.clone())); // Some(None) -> NULL
    }
    if let Some(v) = patch.executor_spec.as_ref() {
        sets.push("executor_spec = ?");
        args.push(Arg::TextOrNull(v.clone())); // Some(None) -> NULL
    }
    if let Some(v) = patch.milestone_id.as_ref() {
        sets.push("milestone_id = ?");
        args.push(Arg::TextOrNull(v.clone()));
    }
    if let Some(v) = patch.active_session_id.as_ref() {
        sets.push("active_session_id = ?");
        args.push(Arg::TextOrNull(v.clone()));
    }
    (sets, args)
}

/// Dynamic `SET` from the patch's `Some` fields; always stamps
/// `updated_at = now_ms`. Returns `false` when the id does not exist.
pub async fn patch_todo(
    pool: &MySqlPool,
    starrocks: bool,
    id: &str,
    patch: &crate::project_types::ProjectTodoPatch,
    now_ms: i64,
) -> Result<bool> {
    let (mut sets, mut args) = todo_set_fragment(patch);
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

pub async fn get_todo_summary(
    pool: &MySqlPool,
    starrocks: bool,
    id: &str,
) -> Result<Option<crate::ProjectTodoSummary>> {
    let row = exec_read_opt(
        pool,
        starrocks,
        "SELECT id,milestone_id,title, \
         CASE WHEN OCTET_LENGTH(draft)<=65536 THEN draft END AS draft, \
         OCTET_LENGTH(draft) AS draft_bytes, \
         CASE WHEN OCTET_LENGTH(plan_md)<=65536 THEN plan_md END AS plan_md, \
         OCTET_LENGTH(plan_md) AS plan_bytes,status,agent,active_session_id,created_at,updated_at, \
         executor_kind,executor_ref \
         FROM project_todos WHERE id=? LIMIT 1",
        &[Arg::Text(id.to_string())],
    )
    .await?;
    row.as_ref().map(row_to_todo_summary).transpose()
}

/// Expected-status CAS (`SET status = 'running' WHERE id = ? AND status <>
/// 'running'`): exactly one concurrent caller can flip a todo into running.
/// `false` = not found or already running; both mean "no claim". Note the
/// matched-rows/changed-rows ambiguity of MySQL's affected count does not
/// apply: the WHERE clause guarantees any matched row is also changed (it
/// was not 'running' before).
pub async fn claim_todo_running(
    pool: &MySqlPool,
    starrocks: bool,
    id: &str,
    now_ms: i64,
) -> Result<bool> {
    let running = ProjectTodoStatus::Running.as_str().to_string();
    let n = exec_write(
        pool,
        starrocks,
        "UPDATE project_todos SET status = ?, updated_at = ? WHERE id = ? AND status <> ?",
        vec![
            Arg::Text(running.clone()),
            Arg::Int(now_ms),
            Arg::Text(id.to_string()),
            Arg::Text(running),
        ],
    )
    .await
    .context("claim project todo running")?;
    Ok(n > 0)
}

/// Expected-status CAS variant of `patch_todo`: `WHERE id = ? AND status = ?`.
/// `false` = not found or the state moved on (someone else won the write).
/// MySQL's affected-rows count reports changed rows only, but the
/// `WHERE status = ?` guard means any matched row is also changed PROVIDED
/// the patched status differs from `when` — callers must never set
/// `patch.status == when` or a no-op SET would read as a lost CAS.
pub async fn patch_todo_when(
    pool: &MySqlPool,
    starrocks: bool,
    id: &str,
    when: ProjectTodoStatus,
    patch: &crate::project_types::ProjectTodoPatch,
    now_ms: i64,
) -> Result<bool> {
    let (mut sets, mut args) = todo_set_fragment(patch);
    sets.push("updated_at = ?");
    args.push(Arg::Int(now_ms));
    let sql = format!(
        "UPDATE project_todos SET {} WHERE id = ? AND status = ?",
        sets.join(", ")
    );
    args.push(Arg::Text(id.to_string()));
    args.push(Arg::Text(when.as_str().to_string()));
    let n = exec_write(pool, starrocks, &sql, args)
        .await
        .context("patch project todo (expected status)")?;
    Ok(n > 0)
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
    let executor_kind: String = r.try_get("executor_kind")?;
    Ok(ProjectTodoRecord {
        id: r.try_get("id")?,
        milestone_id: r.try_get::<Option<String>, _>("milestone_id")?,
        title: r.try_get("title")?,
        draft: r.try_get("draft")?,
        plan_md: r.try_get::<Option<String>, _>("plan_md")?,
        status: ProjectTodoStatus::parse(&status)
            .ok_or_else(|| corrupt_status("project_todos.status", &status))?,
        agent: r.try_get("agent")?,
        executor_kind: ProjectExecutorKind::parse(&executor_kind).ok_or_else(|| {
            anyhow::anyhow!("corrupt project row: unknown executor kind {executor_kind}")
        })?,
        executor_ref: r.try_get::<Option<String>, _>("executor_ref")?,
        executor_spec: r.try_get::<Option<String>, _>("executor_spec")?,
        active_session_id: r.try_get::<Option<String>, _>("active_session_id")?,
        created_at: r.try_get("created_at")?,
        updated_at: r.try_get("updated_at")?,
    })
}

fn row_to_todo_summary(r: &sqlx::mysql::MySqlRow) -> Result<crate::ProjectTodoSummary> {
    let id: String = r.try_get("id")?;
    let status: String = r.try_get("status")?;
    let executor_kind: String = r.try_get("executor_kind")?;
    Ok(crate::ProjectTodoSummary {
        id: id.clone(),
        milestone_id: r.try_get("milestone_id")?,
        title: r.try_get("title")?,
        draft: todo_text(r.try_get("draft")?, r.try_get("draft_bytes")?, &id, "draft")
            .ok_or_else(|| anyhow::anyhow!("corrupt project todo: null draft"))?,
        plan_md: todo_text(
            r.try_get("plan_md")?,
            r.try_get("plan_bytes")?,
            &id,
            "plan_md",
        ),
        status: ProjectTodoStatus::parse(&status)
            .ok_or_else(|| corrupt_status("project_todos.status", &status))?,
        agent: r.try_get("agent")?,
        executor_kind: ProjectExecutorKind::parse(&executor_kind).ok_or_else(|| {
            anyhow::anyhow!("corrupt project row: unknown executor kind {executor_kind}")
        })?,
        executor_ref: r.try_get("executor_ref")?,
        active_session_id: r.try_get("active_session_id")?,
        created_at: r.try_get("created_at")?,
        updated_at: r.try_get("updated_at")?,
    })
}

fn todo_text(
    value: Option<String>,
    bytes: Option<i64>,
    todo_id: &str,
    field: &str,
) -> Option<crate::ProjectRunText> {
    bytes.map(|bytes| {
        if bytes > 64 * 1024 {
            crate::ProjectRunText::Omitted {
                omitted: true,
                total_bytes: bytes.max(0) as u64,
                read_via: "detail_field",
                field: format!("project.todo.{todo_id}.{field}"),
            }
        } else {
            crate::ProjectRunText::Text(value.unwrap_or_default())
        }
    })
}
