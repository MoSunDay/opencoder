//! Project-module persistence — todo-run CRUD (MySQL dialect).
//!
//! The run half of the project tables; companions are
//! [`super::project_crud_todo`] (todos) and [`super::project_crud`]
//! (goals & milestones). Behavior mirrors `libsql_store::project_runs`:
//! dynamic SET patches, newest-first run listing,
//! `COALESCE(MAX(version), 0) + 1` versioning.

use anyhow::{Context, Result};
use sqlx::{MySqlPool, Row};

use super::{bind_args, corrupt_status, exec_read_all, exec_read_opt, exec_write, Arg};
use crate::project_types::{
    ProjectExecutorKind, ProjectTodoRunKind, ProjectTodoRunPatch, ProjectTodoRunRecord,
    ProjectTodoRunStatus,
};

const RUN_COLS: &str = "id, todo_id, kind, version, plan_md, output_md, agent, session_id, status, started_at, finished_at, created_at, executor_kind, capability_id, plan_id, output_ref, input_snapshot, trace_manifest";
const INSERT_RUN: &str = "INSERT INTO project_todo_runs \
    (id, todo_id, kind, version, plan_md, output_md, agent, session_id, status, started_at, finished_at, created_at, executor_kind, capability_id, plan_id, output_ref, input_snapshot, trace_manifest) \
    VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)";

fn validate_claim_run(rec: &ProjectTodoRunRecord) -> Result<()> {
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
        Arg::Text(rec.executor_kind.as_str().to_string()),
        Arg::TextOrNull(rec.capability_id.clone()),
        Arg::TextOrNull(rec.plan_id.clone()),
        Arg::TextOrNull(rec.output_ref.clone()),
        Arg::TextOrNull(rec.input_snapshot.clone()),
        Arg::TextOrNull(rec.trace_manifest.clone()),
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
    let row = sqlx::query("SELECT status FROM project_todos WHERE id=? FOR UPDATE")
        .bind(&rec.todo_id)
        .fetch_optional(&mut *tx)
        .await?;
    let Some(row) = row else {
        tx.rollback().await?;
        return Ok(false);
    };
    if row.try_get::<String, _>("status")? == "running" {
        tx.rollback().await?;
        return Ok(false);
    }
    if sqlx::query("SELECT id FROM project_todo_runs WHERE todo_id=? AND status='running' LIMIT 1")
        .bind(&rec.todo_id)
        .fetch_optional(&mut *tx)
        .await?
        .is_some()
    {
        tx.rollback().await?;
        return Ok(false);
    }
    let next = sqlx::query(
        "SELECT COALESCE(MAX(version),0)+1 AS next_version FROM project_todo_runs WHERE todo_id=?",
    )
    .bind(&rec.todo_id)
    .fetch_one(&mut *tx)
    .await?;
    let mut accepted = rec.clone();
    accepted.version = next.try_get("next_version")?;
    if rec.kind == ProjectTodoRunKind::Execute {
        sqlx::query("UPDATE project_todos SET status='running', updated_at=? WHERE id=?")
            .bind(now_ms)
            .bind(&rec.todo_id)
            .execute(&mut *tx)
            .await?;
    }
    let rec = &accepted;
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
    if let Some(v) = &patch.input_snapshot {
        sets.push("input_snapshot = ?");
        args.push(Arg::Text(v.clone()));
    }
    if let Some(v) = &patch.trace_manifest {
        sets.push("trace_manifest = ?");
        args.push(Arg::Text(v.clone()));
    }
    if let Some(v) = &patch.plan_md {
        sets.push("plan_md = ?");
        args.push(Arg::Text(v.clone()));
    }
    if let Some(v) = &patch.output_md {
        sets.push("output_md = ?");
        args.push(Arg::Text(v.clone()));
    }
    if let Some(v) = &patch.output_ref {
        sets.push("output_ref = ?");
        args.push(Arg::Text(v.clone()));
    }
    if let Some(v) = &patch.capability_id {
        sets.push("capability_id = ?");
        args.push(Arg::Text(v.clone()));
    }
    if let Some(v) = &patch.plan_id {
        sets.push("plan_id = ?");
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

pub async fn finish_todo_run(
    pool: &MySqlPool,
    starrocks: bool,
    id: &str,
    patch: &ProjectTodoRunPatch,
    now_ms: i64,
) -> Result<bool> {
    anyhow::ensure!(
        !starrocks,
        "starrocks does not support atomic project run finalization"
    );
    let mut tx = pool.begin().await?;
    let row =
        sqlx::query("SELECT todo_id,kind,status FROM project_todo_runs WHERE id=? FOR UPDATE")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
    if row.try_get::<String, _>("status")? != "running" {
        tx.rollback().await?;
        return Ok(false);
    }
    let todo: String = row.try_get("todo_id")?;
    let kind: String = row.try_get("kind")?;
    let status = patch.status.context("final run status missing")?;
    anyhow::ensure!(
        status != ProjectTodoRunStatus::Running,
        "run finalization requires terminal status"
    );
    // Step runs (playbook child attempts) never write the todo status —
    // the parent playbook Execute run owns the todo lifecycle. Plan/Execute
    // semantics are unchanged.
    let next = if kind == "plan" {
        (status == ProjectTodoRunStatus::Done).then_some("planned")
    } else if kind == "step" {
        None
    } else {
        Some(match status {
            ProjectTodoRunStatus::Done => "done",
            ProjectTodoRunStatus::Cancelled => "planned",
            _ => "failed",
        })
    };
    if let Some(next) = next {
        anyhow::ensure!(
            sqlx::query("SELECT id FROM project_todos WHERE id=? FOR UPDATE")
                .bind(&todo)
                .fetch_optional(&mut *tx)
                .await?
                .is_some(),
            "todo missing during finalization"
        );
        sqlx::query("UPDATE project_todos SET status=?,updated_at=?,plan_md=CASE WHEN ?='plan' THEN ? ELSE plan_md END WHERE id=? AND (?='plan' OR status='running')").bind(next).bind(now_ms).bind(&kind).bind(&patch.output_md).bind(todo).bind(&kind).execute(&mut *tx).await?;
    }
    let (sets, mut args) = run_set_fragment(patch);
    args.push(Arg::Text(id.into()));
    bind_args(
        sqlx::query(&format!(
            "UPDATE project_todo_runs SET {} WHERE id=?",
            sets.join(", ")
        )),
        args,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(true)
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
         OCTET_LENGTH(output_md) AS output_bytes,agent,session_id,status,started_at,finished_at,created_at, \
         executor_kind,capability_id,plan_id,output_ref, \
         CASE WHEN OCTET_LENGTH(input_snapshot)<=65536 THEN input_snapshot END AS input_snapshot, OCTET_LENGTH(input_snapshot) AS input_bytes, \
         CASE WHEN OCTET_LENGTH(trace_manifest)<=65536 THEN trace_manifest END AS trace_manifest, OCTET_LENGTH(trace_manifest) AS trace_bytes \
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
         OCTET_LENGTH(output_md) AS output_bytes,agent,session_id,status,started_at,finished_at,created_at, \
         executor_kind,capability_id,plan_id,output_ref, \
         CASE WHEN OCTET_LENGTH(input_snapshot)<=65536 THEN input_snapshot END AS input_snapshot, OCTET_LENGTH(input_snapshot) AS input_bytes, \
         CASE WHEN OCTET_LENGTH(trace_manifest)<=65536 THEN trace_manifest END AS trace_manifest, OCTET_LENGTH(trace_manifest) AS trace_bytes \
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
    let out = rows
        .iter()
        .map(row_to_run_summary)
        .collect::<Result<Vec<_>>>()?;
    crate::project_types::project_run_page(out, limit)
}

// Arg list mirrors the `ProjectStore::project_text_chunk` trait method —
// the public seam fixes the arity, so the lint is allowed here.
#[allow(clippy::too_many_arguments)]
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
        ("run", "input_snapshot") => ("project_todo_runs", "input_snapshot"),
        ("run", "trace_manifest") => ("project_todo_runs", "trace_manifest"),
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
    let executor_kind: String = r.try_get("executor_kind")?;
    Ok(ProjectTodoRunRecord {
        input_snapshot: r.try_get("input_snapshot")?,
        trace_manifest: r.try_get("trace_manifest")?,
        id: r.try_get("id")?,
        todo_id: r.try_get("todo_id")?,
        kind: ProjectTodoRunKind::parse(&kind)
            .ok_or_else(|| anyhow::anyhow!("corrupt project row: unknown kind {kind}"))?,
        version: r.try_get("version")?,
        plan_md: r.try_get::<Option<String>, _>("plan_md")?,
        output_md: r.try_get::<Option<String>, _>("output_md")?,
        agent: r.try_get("agent")?,
        executor_kind: ProjectExecutorKind::parse(&executor_kind).ok_or_else(|| {
            anyhow::anyhow!("corrupt project row: unknown executor kind {executor_kind}")
        })?,
        capability_id: r.try_get::<Option<String>, _>("capability_id")?,
        plan_id: r.try_get::<Option<String>, _>("plan_id")?,
        output_ref: r.try_get::<Option<String>, _>("output_ref")?,
        session_id: r.try_get::<Option<String>, _>("session_id")?,
        status: ProjectTodoRunStatus::parse(&status)
            .ok_or_else(|| corrupt_status("project_todo_runs.status", &status))?,
        started_at: r.try_get("started_at")?,
        finished_at: r.try_get::<Option<i64>, _>("finished_at")?,
        created_at: r.try_get("created_at")?,
    })
}

fn row_to_run_summary(r: &sqlx::mysql::MySqlRow) -> Result<crate::ProjectTodoRunSummary> {
    let kind: String = r.try_get("kind")?;
    let status: String = r.try_get("status")?;
    let id: String = r.try_get("id")?;
    let executor_kind: String = r.try_get("executor_kind")?;
    Ok(crate::ProjectTodoRunSummary {
        input_snapshot: summary_text(
            r.try_get("input_snapshot")?,
            r.try_get("input_bytes")?,
            &id,
            "input_snapshot",
        ),
        trace_manifest: summary_text(
            r.try_get("trace_manifest")?,
            r.try_get("trace_bytes")?,
            &id,
            "trace_manifest",
        ),
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
        executor_kind: ProjectExecutorKind::parse(&executor_kind).ok_or_else(|| {
            anyhow::anyhow!("corrupt project row: unknown executor kind {executor_kind}")
        })?,
        capability_id: r.try_get("capability_id")?,
        plan_id: r.try_get("plan_id")?,
        output_ref: r.try_get("output_ref")?,
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
            input_snapshot: None,
            trace_manifest: None,
            id: "run".into(),
            todo_id: "todo".into(),
            kind: ProjectTodoRunKind::Execute,
            version: 1,
            plan_md: Some("plan".into()),
            output_md: None,
            agent: "act".into(),
            executor_kind: ProjectExecutorKind::Agent,
            capability_id: None,
            plan_id: None,
            output_ref: None,
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
