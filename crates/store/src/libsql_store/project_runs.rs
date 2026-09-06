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

/// The `SET` clause fragments + bound values shared by `patch_todo` and its
/// expected-status CAS variant — pure projection of the patch's `Some`
/// fields, no I/O. `Option<Option<String>>` fields distinguish "leave
/// unchanged" (outer `None`) from "clear to NULL" (`Some(None)`).
fn todo_set_fragment(
    patch: &crate::project_types::ProjectTodoPatch,
) -> (Vec<&'static str>, Vec<Value>) {
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
    (sets, vals)
}

/// Dynamic `SET` from the patch's `Some` fields; always stamps
/// `updated_at = now_ms`. Returns `false` when the id does not exist.
pub async fn patch_todo(
    conn: &Connection,
    id: &str,
    patch: &crate::project_types::ProjectTodoPatch,
    now_ms: i64,
) -> Result<bool> {
    let (mut sets, mut vals) = todo_set_fragment(patch);
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

pub async fn get_todo_summary(
    conn: &Connection,
    id: &str,
) -> Result<Option<crate::ProjectTodoSummary>> {
    let mut rows = conn
        .query(
            "SELECT id,milestone_id,title, \
             CASE WHEN length(CAST(draft AS BLOB))<=65536 THEN draft END, \
             length(CAST(draft AS BLOB)), \
             CASE WHEN length(CAST(plan_md AS BLOB))<=65536 THEN plan_md END, \
             length(CAST(plan_md AS BLOB)),status,agent,active_session_id,created_at,updated_at \
             FROM project_todos WHERE id=?1 LIMIT 1",
            params![id],
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    let todo_id: String = row.get(0)?;
    Ok(Some(crate::ProjectTodoSummary {
        id: todo_id.clone(),
        milestone_id: row.get(1)?,
        title: row.get(2)?,
        draft: todo_text(row.get(3)?, row.get(4)?, &todo_id, "draft")
            .context("project todo draft is null")?,
        plan_md: todo_text(row.get(5)?, row.get(6)?, &todo_id, "plan_md"),
        status: ProjectTodoStatus::parse(&row.get::<String>(7)?).context("project_todos.status")?,
        agent: row.get(8)?,
        active_session_id: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
    }))
}

/// Expected-status CAS (`SET status = 'running' WHERE id = ? AND status <>
/// 'running'`): exactly one concurrent caller can flip a todo into running.
/// `false` = not found or already running; both mean "no claim".
pub async fn claim_todo_running(conn: &Connection, id: &str, now_ms: i64) -> Result<bool> {
    let running = ProjectTodoStatus::Running.as_str();
    let n = conn
        .execute(
            "UPDATE project_todos SET status = ?1, updated_at = ?2 WHERE id = ?3 AND status <> ?1",
            params![running, now_ms, id],
        )
        .await
        .context("claim project todo running")?;
    Ok(n > 0)
}

fn validate_claim_run(rec: &ProjectTodoRunRecord) -> Result<()> {
    anyhow::ensure!(
        rec.kind == ProjectTodoRunKind::Execute,
        "atomic todo claim requires an execute run"
    );
    anyhow::ensure!(
        rec.status == ProjectTodoRunStatus::Running,
        "atomic todo claim requires a running run"
    );
    Ok(())
}

/// One SQLite write transaction owns both the conditional claim and run
/// insert. `BEGIN IMMEDIATE` also serializes separate store connections, so a
/// concurrent caller cannot observe a claimed todo before its run exists.
pub async fn claim_todo_running_with_run(
    conn: &Connection,
    rec: &ProjectTodoRunRecord,
    now_ms: i64,
) -> Result<bool> {
    validate_claim_run(rec)?;
    super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        let running = ProjectTodoStatus::Running.as_str();
        let claimed = conn
            .execute(
                "UPDATE project_todos SET status = ?1, updated_at = ?2 WHERE id = ?3 AND status <> ?1",
                params![running, now_ms, rec.todo_id.as_str()],
            )
            .await
            .context("claim project todo running")?;
        if claimed == 0 {
            return Ok(false);
        }
        create_todo_run(conn, rec).await?;
        Ok(true)
    })
    .await
}

/// Expected-status CAS variant of `patch_todo`: `WHERE id = ? AND status = ?`.
/// Applies only while the row still holds `when`; `false` = not found or the
/// state moved on (someone else won the write).
pub async fn patch_todo_when(
    conn: &Connection,
    id: &str,
    when: ProjectTodoStatus,
    patch: &crate::project_types::ProjectTodoPatch,
    now_ms: i64,
) -> Result<bool> {
    let (mut sets, mut vals) = todo_set_fragment(patch);
    sets.push("updated_at = ?");
    vals.push(now_ms.into());
    let sql = format!(
        "UPDATE project_todos SET {} WHERE id = ? AND status = ?",
        sets.join(", ")
    );
    vals.push(id.into());
    vals.push(when.as_str().into());
    let n = conn
        .execute(&sql, vals)
        .await
        .context("patch project todo (expected status)")?;
    Ok(n > 0)
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

/// The `SET` clause fragments + bound values shared by `patch_todo_run` and
/// its expected-status CAS variant — pure projection of the patch's `Some`
/// fields, no I/O. Plain `Option<String>` fields set, never clear.
fn run_set_fragment(patch: &ProjectTodoRunPatch) -> (Vec<&'static str>, Vec<Value>) {
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
    (sets, vals)
}

pub async fn patch_todo_run(
    conn: &Connection,
    id: &str,
    patch: &ProjectTodoRunPatch,
    _now_ms: i64,
) -> Result<bool> {
    let (sets, mut vals) = run_set_fragment(patch);
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

/// Expected-status CAS variant of `patch_todo_run`: `WHERE id = ? AND
/// status = ?`. Applies only while the run row still holds `when` — a
/// stale convergence must not relabel a row the driver already closed.
pub async fn patch_todo_run_when(
    conn: &Connection,
    id: &str,
    when: ProjectTodoRunStatus,
    patch: &ProjectTodoRunPatch,
    _now_ms: i64,
) -> Result<bool> {
    let (sets, mut vals) = run_set_fragment(patch);
    // Runs have no updated_at; `_now_ms` stays unused for signature
    // uniformity with the other patch_* methods.
    let sql = format!(
        "UPDATE project_todo_runs SET {} WHERE id = ? AND status = ?",
        sets.join(", ")
    );
    vals.push(id.into());
    vals.push(when.as_str().into());
    let n = conn
        .execute(&sql, vals)
        .await
        .context("patch project todo run (expected status)")?;
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

pub async fn get_todo_run_summary(
    conn: &Connection,
    id: &str,
) -> Result<Option<crate::ProjectTodoRunSummary>> {
    let mut rows = conn
        .query(
            "SELECT id,todo_id,kind,version, \
             CASE WHEN length(CAST(plan_md AS BLOB))<=65536 THEN plan_md END, \
             length(CAST(plan_md AS BLOB)), \
             CASE WHEN length(CAST(output_md AS BLOB))<=65536 THEN output_md END, \
             length(CAST(output_md AS BLOB)),agent,session_id,status,started_at,finished_at,created_at \
             FROM project_todo_runs WHERE id=?1 LIMIT 1",
            params![id],
        )
        .await?;
    rows.next()
        .await?
        .map(|row| row_to_run_summary(&row))
        .transpose()
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

pub async fn list_todo_runs_page(
    conn: &Connection,
    todo_id: &str,
    before_version: Option<i64>,
    limit: u32,
) -> Result<crate::ProjectTodoRunPage> {
    let limit = limit.clamp(1, 100) as usize;
    let mut rows = conn
        .query(
            "SELECT id,todo_id,kind,version, \
             CASE WHEN length(CAST(plan_md AS BLOB))<=65536 THEN plan_md END, \
             length(CAST(plan_md AS BLOB)), \
             CASE WHEN length(CAST(output_md AS BLOB))<=65536 THEN output_md END, \
             length(CAST(output_md AS BLOB)),agent,session_id,status,started_at,finished_at,created_at \
             FROM project_todo_runs WHERE todo_id=?1 AND (?2 IS NULL OR version<?2) \
             ORDER BY version DESC LIMIT ?3",
            params![todo_id, before_version, limit as i64 + 1],
        )
        .await?;
    let mut out = Vec::with_capacity(limit + 1);
    while let Some(row) = rows.next().await? {
        out.push(row_to_run_summary(&row)?);
    }
    let more = out.len() > limit;
    out.truncate(limit);
    Ok(crate::ProjectTodoRunPage {
        next_version: more.then(|| out.last().unwrap().version),
        runs: out,
    })
}

pub async fn project_text_chunk(
    conn: &Connection,
    record_kind: &str,
    owner_id: &str,
    id: &str,
    field: &str,
    offset: u64,
    max_bytes: usize,
) -> Result<Option<crate::PayloadChunkRecord>> {
    let (table, column) = match (record_kind, field) {
        ("todo", "draft") => ("project_todos", "draft"),
        ("todo", "plan_md") => ("project_todos", "plan_md"),
        ("run", "plan_md") => ("project_todo_runs", "plan_md"),
        ("run", "output_md") => ("project_todo_runs", "output_md"),
        _ => anyhow::bail!("unsupported project text field"),
    };
    let start = i64::try_from(offset)?.saturating_add(1);
    let take = max_bytes.clamp(1, 64 * 1024) as i64;
    let owner_clause = if record_kind == "run" {
        " AND todo_id=?4"
    } else {
        " AND id=?4"
    };
    let mut rows = conn
        .query(
            &format!(
                "SELECT length(CAST({column} AS BLOB)), \
                 CAST(substr(CAST({column} AS BLOB),?2,?3) AS BLOB) FROM {table} \
                 WHERE id=?1{owner_clause}"
            ),
            params![id, start, take, owner_id],
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    let Some(total) = row.get::<Option<i64>>(0)? else {
        return Ok(None);
    };
    let total = total.max(0) as u64;
    if offset > total {
        anyhow::bail!("project text offset exceeds total bytes");
    }
    Ok(Some(crate::PayloadChunkRecord {
        total_bytes: total,
        bytes: row.get::<Option<Vec<u8>>>(1)?.unwrap_or_default(),
    }))
}

/// Every run row currently in the `running` state (any todo, any kind) —
/// feeds the opportunistic stale-run sweep.
pub async fn list_running_todo_runs(conn: &Connection) -> Result<Vec<ProjectTodoRunRecord>> {
    let stmt = conn
        .prepare(&format!(
            "SELECT {RUN_COLS} FROM project_todo_runs WHERE status = ?1"
        ))
        .await?;
    let mut rows = stmt
        .query(params![ProjectTodoRunStatus::Running.as_str()])
        .await?;
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

fn row_to_run_summary(r: &libsql::Row) -> Result<crate::ProjectTodoRunSummary> {
    let id: String = r.get(0)?;
    Ok(crate::ProjectTodoRunSummary {
        id: id.clone(),
        todo_id: r.get(1)?,
        kind: ProjectTodoRunKind::parse(&r.get::<String>(2)?).context("project_todo_runs.kind")?,
        version: r.get(3)?,
        plan_md: summary_text(r.get(4)?, r.get(5)?, &id, "plan_md"),
        output_md: summary_text(r.get(6)?, r.get(7)?, &id, "output_md"),
        agent: r.get(8)?,
        session_id: r.get(9)?,
        status: ProjectTodoRunStatus::parse(&r.get::<String>(10)?)
            .context("project_todo_runs.status")?,
        started_at: r.get(11)?,
        finished_at: r.get(12)?,
        created_at: r.get(13)?,
    })
}

fn summary_text(
    value: Option<String>,
    bytes: Option<i64>,
    run_id: &str,
    field: &str,
) -> Option<crate::ProjectRunText> {
    bytes.map(|bytes| {
        if bytes > 64 * 1024 {
            crate::ProjectRunText::Omitted {
                omitted: true,
                total_bytes: bytes.max(0) as u64,
                read_via: "detail_field",
                field: format!("project.run.{run_id}.{field}"),
            }
        } else {
            crate::ProjectRunText::Text(value.unwrap_or_default())
        }
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
