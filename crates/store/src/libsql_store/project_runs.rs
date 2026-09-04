//! Project-module persistence — todos & todo runs CRUD (libsql).
//!
//! Companion of [`super::project`] (which holds goals/milestones and the
//! `ProjectStore` impl). Free functions over a raw `Connection`; deletes run
//! via [`super::tx::run_tx`] (`BEGIN IMMEDIATE`) with explicit cascades.

use anyhow::{Context, Result};
use libsql::{params, Connection, Value};

use crate::project_types::{
    ProjectTodoRecord, ProjectTodoRunKind, ProjectTodoRunPatch, ProjectTodoRunRecord,
    ProjectTodoRunStatus, ProjectTodoStatus,
};

const TODO_COLS: &str = "id, milestone_id, title, draft, plan_md, status, agent, active_session_id, created_at, updated_at";
const RUN_COLS: &str = "id, todo_id, kind, version, plan_md, output_md, agent, session_id, status, started_at, finished_at, created_at";

// ---- todos ----

pub async fn create_todo(conn: &Connection, rec: &ProjectTodoRecord) -> Result<()> {
    conn.execute(
        "INSERT INTO project_todos (id, milestone_id, title, draft, plan_md, status, agent, active_session_id, created_at, updated_at) VALUES (?,?,?,?,?,?,?,?,?,?)",
        params![
            rec.id.as_str(),
            rec.milestone_id.as_deref(),
            rec.title.as_str(),
            rec.draft.as_str(),
            rec.plan_md.as_deref(),
            rec.status.as_str(),
            rec.agent.as_str(),
            rec.active_session_id.as_deref(),
            rec.created_at,
            rec.updated_at
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
    conn: &Connection,
    id: &str,
    patch: &crate::project_types::ProjectTodoPatch,
    now_ms: i64,
) -> Result<bool> {
    let mut sets: Vec<&'static str> = Vec::new();
    let mut vals: Vec<Value> = Vec::new();
    if let Some(v) = patch.title.as_deref() {
        sets.push("title = ?");
        vals.push(v.into());
    }
    if let Some(v) = patch.draft.as_deref() {
        sets.push("draft = ?");
        vals.push(v.into());
    }
    if let Some(v) = patch.plan_md.as_ref() {
        sets.push("plan_md = ?");
        vals.push(v.as_deref().into()); // Some(None) -> NULL
    }
    if let Some(v) = patch.status {
        sets.push("status = ?");
        vals.push(v.as_str().into());
    }
    if let Some(v) = patch.agent.as_deref() {
        sets.push("agent = ?");
        vals.push(v.into());
    }
    if let Some(v) = patch.milestone_id.as_ref() {
        sets.push("milestone_id = ?");
        vals.push(v.as_deref().into());
    }
    if let Some(v) = patch.active_session_id.as_ref() {
        sets.push("active_session_id = ?");
        vals.push(v.as_deref().into());
    }
    sets.push("updated_at = ?");
    vals.push(now_ms.into());
    let sql = format!("UPDATE project_todos SET {} WHERE id = ?", sets.join(", "));
    vals.push(id.into());
    let n = conn
        .execute(&sql, vals)
        .await
        .context("patch project todo")?;
    Ok(n > 0)
}

/// Tx cascade: the todo's runs, then the todo. `false` when the id does not
/// exist.
pub async fn delete_todo(conn: &Connection, id: &str) -> Result<bool> {
    super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        let exists = {
            let stmt = conn
                .prepare("SELECT 1 FROM project_todos WHERE id = ?1")
                .await?;
            let mut rows = stmt.query(params![id]).await?;
            rows.next().await?.is_some()
        };
        if !exists {
            return Ok(false);
        }
        conn.execute(
            "DELETE FROM project_todo_runs WHERE todo_id = ?1",
            params![id],
        )
        .await
        .context("cascade delete todo runs")?;
        conn.execute("DELETE FROM project_todos WHERE id = ?1", params![id])
            .await
            .context("delete project todo")?;
        Ok(true)
    })
    .await
}

pub async fn get_todo(conn: &Connection, id: &str) -> Result<Option<ProjectTodoRecord>> {
    let stmt = conn
        .prepare(&format!(
            "SELECT {TODO_COLS} FROM project_todos WHERE id = ?1 LIMIT 1"
        ))
        .await?;
    let mut rows = stmt.query(params![id]).await?;
    match rows.next().await? {
        Some(r) => Ok(Some(row_to_todo(&r)?)),
        None => Ok(None),
    }
}

/// `milestone_id == None` lists ALL todos (backlog included); ordered by
/// `created_at`.
pub async fn list_todos(
    conn: &Connection,
    milestone_id: Option<&str>,
) -> Result<Vec<ProjectTodoRecord>> {
    let mut sql = format!("SELECT {TODO_COLS} FROM project_todos");
    if milestone_id.is_some() {
        sql.push_str(" WHERE milestone_id = ?");
    }
    sql.push_str(" ORDER BY created_at");
    let stmt = conn.prepare(&sql).await?;
    let mut rows = match milestone_id {
        Some(m) => stmt.query(params![m]).await?,
        None => stmt.query(()).await?,
    };
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        out.push(row_to_todo(&r)?);
    }
    Ok(out)
}

fn row_to_todo(r: &libsql::Row) -> Result<ProjectTodoRecord> {
    Ok(ProjectTodoRecord {
        id: r.get(0)?,
        milestone_id: r.get(1)?,
        title: r.get(2)?,
        draft: r.get(3)?,
        plan_md: r.get(4)?,
        // Unparseable status/kind is corruption: propagate, never coerce.
        status: ProjectTodoStatus::parse(&r.get::<String>(5)?).context("project_todos.status")?,
        agent: r.get(6)?,
        active_session_id: r.get(7)?,
        created_at: r.get(8)?,
        updated_at: r.get(9)?,
    })
}

// ---- todo runs ----

pub async fn create_todo_run(conn: &Connection, rec: &ProjectTodoRunRecord) -> Result<()> {
    conn.execute(
        "INSERT INTO project_todo_runs (id, todo_id, kind, version, plan_md, output_md, agent, session_id, status, started_at, finished_at, created_at) VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
        params![
            rec.id.as_str(),
            rec.todo_id.as_str(),
            rec.kind.as_str(),
            rec.version,
            rec.plan_md.as_deref(),
            rec.output_md.as_deref(),
            rec.agent.as_str(),
            rec.session_id.as_deref(),
            rec.status.as_str(),
            rec.started_at,
            rec.finished_at,
            rec.created_at
        ],
    )
    .await
    .context("insert project todo run")?;
    Ok(())
}

pub async fn patch_todo_run(
    conn: &Connection,
    id: &str,
    patch: &ProjectTodoRunPatch,
    _now_ms: i64,
) -> Result<bool> {
    let mut sets: Vec<&'static str> = Vec::new();
    let mut vals: Vec<Value> = Vec::new();
    if let Some(v) = patch.plan_md.as_deref() {
        sets.push("plan_md = ?");
        vals.push(v.into());
    }
    if let Some(v) = patch.output_md.as_deref() {
        sets.push("output_md = ?");
        vals.push(v.into());
    }
    if let Some(v) = patch.session_id.as_deref() {
        sets.push("session_id = ?");
        vals.push(v.into());
    }
    if let Some(v) = patch.status {
        sets.push("status = ?");
        vals.push(v.as_str().into());
    }
    if let Some(v) = patch.finished_at {
        sets.push("finished_at = ?");
        vals.push(v.into());
    }
    // The runs table has no updated_at column (created_at + finished_at span
    // its lifecycle), so the now_ms parameter stays unused; it exists for
    // signature uniformity with the other patch_* methods.
    let sql = format!(
        "UPDATE project_todo_runs SET {} WHERE id = ?",
        sets.join(", ")
    );
    vals.push(id.into());
    let n = conn
        .execute(&sql, vals)
        .await
        .context("patch project todo run")?;
    Ok(n > 0)
}

pub async fn get_todo_run(conn: &Connection, id: &str) -> Result<Option<ProjectTodoRunRecord>> {
    let stmt = conn
        .prepare(&format!(
            "SELECT {RUN_COLS} FROM project_todo_runs WHERE id = ?1 LIMIT 1"
        ))
        .await?;
    let mut rows = stmt.query(params![id]).await?;
    match rows.next().await? {
        Some(r) => Ok(Some(row_to_run(&r)?)),
        None => Ok(None),
    }
}

/// Newest version first.
pub async fn list_todo_runs(conn: &Connection, todo_id: &str) -> Result<Vec<ProjectTodoRunRecord>> {
    let stmt = conn
        .prepare(&format!(
            "SELECT {RUN_COLS} FROM project_todo_runs WHERE todo_id = ?1 ORDER BY version DESC"
        ))
        .await?;
    let mut rows = stmt.query(params![todo_id]).await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        out.push(row_to_run(&r)?);
    }
    Ok(out)
}

/// `COALESCE(MAX(version), 0) + 1` — 1 for a todo with no runs yet.
pub async fn next_todo_version(conn: &Connection, todo_id: &str) -> Result<i64> {
    let stmt = conn
        .prepare("SELECT COALESCE(MAX(version), 0) + 1 FROM project_todo_runs WHERE todo_id = ?1")
        .await
        .context("prepare next_todo_version")?;
    let mut rows = stmt
        .query(params![todo_id])
        .await
        .context("query next_todo_version")?;
    match rows.next().await? {
        Some(r) => Ok(r.get(0)?),
        None => Ok(1), // unreachable: the aggregate always yields a row
    }
}

fn row_to_run(r: &libsql::Row) -> Result<ProjectTodoRunRecord> {
    Ok(ProjectTodoRunRecord {
        id: r.get(0)?,
        todo_id: r.get(1)?,
        kind: ProjectTodoRunKind::parse(&r.get::<String>(2)?).context("project_todo_runs.kind")?,
        version: r.get(3)?,
        plan_md: r.get(4)?,
        output_md: r.get(5)?,
        agent: r.get(6)?,
        session_id: r.get(7)?,
        status: ProjectTodoRunStatus::parse(&r.get::<String>(8)?)
            .context("project_todo_runs.status")?,
        started_at: r.get(9)?,
        finished_at: r.get(10)?,
        created_at: r.get(11)?,
    })
}
