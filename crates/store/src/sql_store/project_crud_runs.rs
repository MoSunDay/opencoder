//! Project-module persistence — todos & todo runs CRUD (MySQL dialect).
//!
//! Companion of [`super::project_crud`] (goals & milestones). Behavior
//! mirrors `libsql_store::project_runs`: dynamic SET patches with
//! `Option<Option<String>>` clearing, newest-first run listing,
//! `COALESCE(MAX(version), 0) + 1` versioning.

use anyhow::{Context, Result};
use sqlx::{MySqlPool, Row};

use super::{
    bind_args, corrupt_status, exec_read_all, exec_read_opt, exec_write, row_exists, run_cascade,
    Arg,
};
use crate::project_types::{
    ProjectTodoRecord, ProjectTodoRunKind, ProjectTodoRunPatch, ProjectTodoRunRecord,
    ProjectTodoRunStatus, ProjectTodoStatus,
};

const TODO_COLS: &str = "id, milestone_id, title, draft, plan_md, status, agent, active_session_id, created_at, updated_at";
const RUN_COLS: &str = "id, todo_id, kind, version, plan_md, output_md, agent, session_id, status, started_at, finished_at, created_at";
const INSERT_RUN: &str = "INSERT INTO project_todo_runs \
    (id, todo_id, kind, version, plan_md, output_md, agent, session_id, status, started_at, finished_at, created_at) \
    VALUES (?,?,?,?,?,?,?,?,?,?,?,?)";

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
         OCTET_LENGTH(plan_md) AS plan_bytes,status,agent,active_session_id,created_at,updated_at \
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

fn run_insert_args(rec: &ProjectTodoRunRecord) -> Vec<Arg> {
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
    ]
}

/// MySQL uses one InnoDB transaction for the claim and run insert. StarRocks
/// has no transaction spanning these primary-key tables, so it fails before
/// either write instead of leaving a claimed todo without a run.
pub async fn claim_todo_running_with_run(
    pool: &MySqlPool,
    starrocks: bool,
    rec: &ProjectTodoRunRecord,
    now_ms: i64,
) -> Result<bool> {
    validate_claim_run(rec)?;
    anyhow::ensure!(
        !starrocks,
        "starrocks does not support atomic project todo claim and run creation"
    );

    let mut tx = pool
        .begin()
        .await
        .context("begin project execute claim tx")?;
    let running = ProjectTodoStatus::Running.as_str().to_string();
    let claimed = bind_args(
        sqlx::query(
            "UPDATE project_todos SET status = ?, updated_at = ? WHERE id = ? AND status <> ?",
        ),
        vec![
            Arg::Text(running.clone()),
            Arg::Int(now_ms),
            Arg::Text(rec.todo_id.clone()),
            Arg::Text(running),
        ],
    )
    .execute(&mut *tx)
    .await
    .context("claim project todo running")?
    .rows_affected();
    if claimed == 0 {
        tx.rollback()
            .await
            .context("rollback lost project execute claim")?;
        return Ok(false);
    }

    if let Err(insert_error) = bind_args(sqlx::query(INSERT_RUN), run_insert_args(rec))
        .execute(&mut *tx)
        .await
    {
        tx.rollback().await.with_context(|| {
            format!("rollback project execute claim after run insert failed: {insert_error}")
        })?;
        return Err(insert_error).context("insert project todo run");
    }
    tx.commit()
        .await
        .context("commit project execute claim tx")?;
    Ok(true)
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
    exec_write(pool, starrocks, INSERT_RUN, run_insert_args(rec))
        .await
        .context("insert project todo run")?;
    Ok(())
}

/// The `SET` fragments + bound args shared by `patch_todo_run` and its
/// expected-status CAS variant — pure projection of the patch's `Some`
/// fields, no I/O. Plain `Option<String>` fields set, never clear.
fn run_set_fragment(patch: &ProjectTodoRunPatch) -> (Vec<&'static str>, Vec<Arg>) {
    let mut sets: Vec<&'static str> = Vec::new();
    let mut args: Vec<Arg> = Vec::new();
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
    (sets, args)
}

pub async fn patch_todo_run(
    pool: &MySqlPool,
    starrocks: bool,
    id: &str,
    patch: &ProjectTodoRunPatch,
    _now_ms: i64,
) -> Result<bool> {
    let (sets, mut args) = run_set_fragment(patch);
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

/// Expected-status CAS variant of `patch_todo_run`: `WHERE id = ? AND
/// status = ?`. `false` = not found or the row is no longer in `when` —
/// a stale convergence must not relabel a row the driver already closed.
/// Same matched-vs-changed caveat as `patch_todo_when` (and the same
/// guarantee from the `WHERE status = ?` guard): any matched row is also
/// changed provided the patched status differs from `when`. Runs have no
/// updated_at, so `_now_ms` stays unused for signature uniformity.
pub async fn patch_todo_run_when(
    pool: &MySqlPool,
    starrocks: bool,
    id: &str,
    when: ProjectTodoRunStatus,
    patch: &ProjectTodoRunPatch,
    _now_ms: i64,
) -> Result<bool> {
    let (sets, mut args) = run_set_fragment(patch);
    let sql = format!(
        "UPDATE project_todo_runs SET {} WHERE id = ? AND status = ?",
        sets.join(", ")
    );
    args.push(Arg::Text(id.to_string()));
    args.push(Arg::Text(when.as_str().to_string()));
    let n = exec_write(pool, starrocks, &sql, args)
        .await
        .context("patch project todo run (expected status)")?;
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

pub async fn get_todo_run_summary(
    pool: &MySqlPool,
    starrocks: bool,
    id: &str,
) -> Result<Option<crate::ProjectTodoRunSummary>> {
    let row = exec_read_opt(
        pool,
        starrocks,
        "SELECT id,todo_id,kind,version, \
         CASE WHEN OCTET_LENGTH(plan_md)<=65536 THEN plan_md END AS plan_md, \
         OCTET_LENGTH(plan_md) AS plan_bytes, \
         CASE WHEN OCTET_LENGTH(output_md)<=65536 THEN output_md END AS output_md, \
         OCTET_LENGTH(output_md) AS output_bytes,agent,session_id,status,started_at,finished_at,created_at \
         FROM project_todo_runs WHERE id=? LIMIT 1",
        &[Arg::Text(id.to_string())],
    )
    .await?;
    row.as_ref().map(row_to_run_summary).transpose()
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

pub async fn list_todo_runs_page(
    pool: &MySqlPool,
    starrocks: bool,
    todo_id: &str,
    before_version: Option<i64>,
    limit: u32,
) -> Result<crate::ProjectTodoRunPage> {
    let limit = limit.clamp(1, 100) as usize;
    let rows = exec_read_all(
        pool,
        starrocks,
        "SELECT id,todo_id,kind,version, \
         CASE WHEN OCTET_LENGTH(plan_md)<=65536 THEN plan_md END AS plan_md, \
         OCTET_LENGTH(plan_md) AS plan_bytes, \
         CASE WHEN OCTET_LENGTH(output_md)<=65536 THEN output_md END AS output_md, \
         OCTET_LENGTH(output_md) AS output_bytes,agent,session_id,status,started_at,finished_at,created_at \
         FROM project_todo_runs WHERE todo_id=? AND (? IS NULL OR version<?) \
         ORDER BY version DESC LIMIT ?",
        &[
            Arg::Text(todo_id.to_string()),
            Arg::IntOrNull(before_version),
            Arg::IntOrNull(before_version),
            Arg::Int(limit as i64 + 1),
        ],
    )
    .await?;
    let mut out = rows
        .iter()
        .map(row_to_run_summary)
        .collect::<Result<Vec<_>>>()?;
    let more = out.len() > limit;
    out.truncate(limit);
    Ok(crate::ProjectTodoRunPage {
        next_version: more.then(|| out.last().unwrap().version),
        runs: out,
    })
}

pub async fn project_text_chunk(
    pool: &MySqlPool,
    starrocks: bool,
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
        " AND todo_id=?"
    } else {
        " AND id=?"
    };
    let row = exec_read_opt(
        pool,
        starrocks,
        &format!(
            "SELECT OCTET_LENGTH({column}) AS total_bytes, \
             SUBSTRING(CAST({column} AS BINARY),?,?) AS bytes FROM {table} \
             WHERE id=?{owner_clause}"
        ),
        &[
            Arg::Int(start),
            Arg::Int(take),
            Arg::Text(id.to_string()),
            Arg::Text(owner_id.to_string()),
        ],
    )
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let Some(total) = row.try_get::<Option<i64>, _>("total_bytes")? else {
        return Ok(None);
    };
    let total = total.max(0) as u64;
    if offset > total {
        anyhow::bail!("project text offset exceeds total bytes");
    }
    Ok(Some(crate::PayloadChunkRecord {
        total_bytes: total,
        bytes: row
            .try_get::<Option<Vec<u8>>, _>("bytes")?
            .unwrap_or_default(),
    }))
}

/// Every run row currently in the `running` state (any todo, any kind) —
/// feeds the opportunistic stale-run sweep.
pub async fn list_running_todo_runs(
    pool: &MySqlPool,
    starrocks: bool,
) -> Result<Vec<ProjectTodoRunRecord>> {
    let rows = exec_read_all(
        pool,
        starrocks,
        &format!("SELECT {RUN_COLS} FROM project_todo_runs WHERE status = ?"),
        &[Arg::Text(
            ProjectTodoRunStatus::Running.as_str().to_string(),
        )],
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

fn row_to_todo_summary(r: &sqlx::mysql::MySqlRow) -> Result<crate::ProjectTodoSummary> {
    let id: String = r.try_get("id")?;
    let status: String = r.try_get("status")?;
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
        active_session_id: r.try_get("active_session_id")?,
        created_at: r.try_get("created_at")?,
        updated_at: r.try_get("updated_at")?,
    })
}

fn row_to_run_summary(r: &sqlx::mysql::MySqlRow) -> Result<crate::ProjectTodoRunSummary> {
    let kind: String = r.try_get("kind")?;
    let status: String = r.try_get("status")?;
    let id: String = r.try_get("id")?;
    Ok(crate::ProjectTodoRunSummary {
        id: id.clone(),
        todo_id: r.try_get("todo_id")?,
        kind: ProjectTodoRunKind::parse(&kind)
            .ok_or_else(|| anyhow::anyhow!("corrupt project row: unknown kind {kind}"))?,
        version: r.try_get("version")?,
        plan_md: summary_text(
            r.try_get("plan_md")?,
            r.try_get("plan_bytes")?,
            &id,
            "plan_md",
        ),
        output_md: summary_text(
            r.try_get("output_md")?,
            r.try_get("output_bytes")?,
            &id,
            "output_md",
        ),
        agent: r.try_get("agent")?,
        session_id: r.try_get("session_id")?,
        status: ProjectTodoRunStatus::parse(&status)
            .ok_or_else(|| corrupt_status("project_todo_runs.status", &status))?,
        started_at: r.try_get("started_at")?,
        finished_at: r.try_get("finished_at")?,
        created_at: r.try_get("created_at")?,
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

#[cfg(test)]
mod tests {
    use sqlx::mysql::MySqlPoolOptions;

    use super::*;

    #[tokio::test]
    async fn starrocks_atomic_claim_refuses_before_connecting() {
        let pool = MySqlPoolOptions::new()
            .connect_lazy("mysql://unused:unused@127.0.0.1:9/unused")
            .unwrap();
        let rec = ProjectTodoRunRecord {
            id: "run".into(),
            todo_id: "todo".into(),
            kind: ProjectTodoRunKind::Execute,
            version: 1,
            plan_md: Some("plan".into()),
            output_md: None,
            agent: "act".into(),
            session_id: None,
            status: ProjectTodoRunStatus::Running,
            started_at: 1,
            finished_at: None,
            created_at: 1,
        };
        let error = claim_todo_running_with_run(&pool, true, &rec, 1)
            .await
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("starrocks does not support atomic"));
    }
}
