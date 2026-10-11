//! TODO records and people; write transactions belong to the caller.
use super::super::project::people;
use super::{
    create_todo_run, next_todo_version, summary::todo_text, todo_patch::todo_set_fragment,
};
use crate::project_types::{
    ProjectExecutorKind, ProjectTodoRecord, ProjectTodoRunKind, ProjectTodoRunRecord,
    ProjectTodoRunStatus, ProjectTodoStatus,
};
use anyhow::{Context, Result};
use libsql::{params, Connection};
const TODO_COLS: &str = "id, initiative_id, title, draft, plan_md, status, agent, active_session_id, created_at, updated_at, executor_kind, executor_ref, executor_spec, board_status, position, capability_id";

// ---- todos ----

pub async fn create_todo(conn: &Connection, rec: &ProjectTodoRecord) -> Result<()> {
    conn.execute(
        "INSERT INTO project_todos (id, initiative_id, title, draft, plan_md, status, agent, active_session_id, created_at, updated_at, executor_kind, executor_ref, executor_spec, board_status, position, capability_id) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        params![
            rec.id.as_str(),
            rec.initiative_id.as_deref(),
            rec.title.as_str(),
            rec.draft.as_str(),
            rec.plan_md.as_deref(),
            rec.status.as_str(),
            rec.agent.as_str(),
            rec.active_session_id.as_deref(),
            rec.created_at,
            rec.updated_at,
            rec.executor_kind.as_str(),
            rec.executor_ref.as_deref(),
            rec.executor_spec.as_deref(),
            rec.board_status.as_str(),
            rec.position,
            rec.capability_id.as_deref()
        ],
    )
    .await
    .context("insert project todo")?;
    people::patch(conn, "todo", &rec.id, &rec.people.replacement()).await?;
    Ok(())
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
    if n > 0 {
        people::patch(conn, "todo", id, &patch.people).await?;
    }
    Ok(n > 0)
}

/// Tx cascade: the todo's runs, then the todo. `false` when the id does not
/// exist.
pub async fn delete_todo(conn: &Connection, id: &str) -> Result<bool> {
    super::super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
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
            "DELETE FROM project_todo_executions WHERE todo_id = ?1",
            params![id],
        )
        .await?;
        conn.execute(
            "DELETE FROM project_todo_runs WHERE todo_id = ?1",
            params![id],
        )
        .await
        .context("cascade delete todo runs")?;
        conn.execute(
            "DELETE FROM project_todo_tags WHERE todo_id = ?1",
            params![id],
        )
        .await?;
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
        Some(r) => {
            let mut record = row_to_todo(&r)?;
            record.people = people::list(conn, "todo", Some(id))
                .await?
                .remove(id)
                .unwrap_or_default();
            Ok(Some(record))
        }
        None => Ok(None),
    }
}

pub async fn get_todo_summary(
    conn: &Connection,
    id: &str,
) -> Result<Option<crate::ProjectTodoSummary>> {
    let mut rows = conn
        .query(
            "SELECT id,initiative_id,title, \
             CASE WHEN length(CAST(draft AS BLOB))<=65536 THEN draft END, \
             length(CAST(draft AS BLOB)), \
             CASE WHEN length(CAST(plan_md AS BLOB))<=65536 THEN plan_md END, \
             length(CAST(plan_md AS BLOB)),status,agent,active_session_id,created_at,updated_at, \
             executor_kind,executor_ref \
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
        initiative_id: row.get(1)?,
        title: row.get(2)?,
        draft: todo_text(row.get(3)?, row.get(4)?, &todo_id, "draft")
            .context("project todo draft is null")?,
        plan_md: todo_text(row.get(5)?, row.get(6)?, &todo_id, "plan_md"),
        status: ProjectTodoStatus::parse(&row.get::<String>(7)?).context("project_todos.status")?,
        agent: row.get(8)?,
        executor_kind: ProjectExecutorKind::parse(&row.get::<String>(12)?)
            .context("project_todos.executor_kind")?,
        executor_ref: row.get(13)?,
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
    super::super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        let mut rows = conn
            .query(
                "SELECT status FROM project_todos WHERE id=?1",
                params![rec.todo_id.as_str()],
            )
            .await?;
        let Some(row) = rows.next().await? else {
            return Ok(false);
        };
        if row.get::<String>(0)? == "running" {
            return Ok(false);
        }
        let mut rows = conn
            .query(
                "SELECT 1 FROM project_todo_runs WHERE todo_id=?1 AND status='running' LIMIT 1",
                params![rec.todo_id.as_str()],
            )
            .await?;
        if rows.next().await?.is_some() {
            return Ok(false);
        }
        let mut accepted = rec.clone();
        accepted.version = next_todo_version(conn, &rec.todo_id).await?;
        if rec.kind == ProjectTodoRunKind::Execute {
            conn.execute(
                "UPDATE project_todos SET status='running', updated_at=?1 WHERE id=?2",
                params![now_ms, rec.todo_id.as_str()],
            )
            .await?;
        }
        let rec = &accepted;
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
    if n > 0 {
        people::patch(conn, "todo", id, &patch.people).await?;
    }
    Ok(n > 0)
}

/// `initiative_id == None` lists ALL todos (backlog included); ordered by
/// `created_at`.
pub async fn list_todos(
    conn: &Connection,
    initiative_id: Option<&str>,
) -> Result<Vec<ProjectTodoRecord>> {
    let mut people = people::list(conn, "todo", None).await?;
    let mut sql = format!("SELECT {TODO_COLS} FROM project_todos");
    if initiative_id.is_some() {
        sql.push_str(" WHERE initiative_id = ?");
    }
    sql.push_str(" ORDER BY board_status, position, created_at, id");
    let stmt = conn.prepare(&sql).await?;
    let mut rows = match initiative_id {
        Some(m) => stmt.query(params![m]).await?,
        None => stmt.query(()).await?,
    };
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        let mut record = row_to_todo(&r)?;
        record.people = people.remove(&record.id).unwrap_or_default();
        out.push(record);
    }
    Ok(out)
}

fn row_to_todo(r: &libsql::Row) -> Result<ProjectTodoRecord> {
    Ok(ProjectTodoRecord {
        people: Default::default(),
        id: r.get(0)?,
        initiative_id: r.get(1)?,
        title: r.get(2)?,
        draft: r.get(3)?,
        plan_md: r.get(4)?,
        // Unparseable status/kind is corruption: propagate, never coerce.
        status: ProjectTodoStatus::parse(&r.get::<String>(5)?).context("project_todos.status")?,
        agent: r.get(6)?,
        executor_kind: ProjectExecutorKind::parse(&r.get::<String>(10)?)
            .context("project_todos.executor_kind")?,
        executor_ref: r.get(11)?,
        executor_spec: r.get(12)?,
        board_status: r.get(13)?,
        position: r.get(14)?,
        capability_id: r.get(15)?,
        active_session_id: r.get(7)?,
        created_at: r.get(8)?,
        updated_at: r.get(9)?,
    })
}
