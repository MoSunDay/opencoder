//! Project-module persistence — goals & initiatives CRUD (turso).
//!
//! Free functions over a raw `Connection`, mirroring sibling submodules;
//! multi-statement deletes run via [`super::tx::run_tx`] (`BEGIN IMMEDIATE`)
//! and cascade explicitly (not via FK tricks) so every backend behaves the
//! same. Todo/run CRUD lives in [`super::project_runs`]; the `ProjectStore`
//! impl at the bottom delegates to both.

use anyhow::{Context, Result};
use turso::{params, Connection, Value};

use super::TursoStore;
#[path = "project/acceptance.rs"]
mod acceptance;
#[path = "project/comments.rs"]
pub(crate) mod comments;
#[path = "project/order.rs"]
mod order;
#[path = "project/people.rs"]
pub(crate) mod people;
#[path = "project/summaries.rs"]
pub(crate) mod summaries;
#[path = "project/tags.rs"]
mod tags;
use crate::project::summary::{SummaryScope, SummaryVersion};
use crate::project::ProjectStore;
use crate::project_types::{
    ProjectGoalPatch, ProjectGoalRecord, ProjectGoalStatus, ProjectInitiativePatch,
    ProjectInitiativeRecord, ProjectTodoPatch, ProjectTodoRecord, ProjectTodoRunPatch,
    ProjectTodoRunRecord, ProjectTodoRunStatus, ProjectTodoStatus,
};

const GOAL_COLS: &str = "id, title, detail_md, status, sort_key, created_at, updated_at, current_summary, summary_updated_at";
const INITIATIVE_COLS: &str =
    "id, goal_id, title, detail_md, sort_key, created_at, updated_at, current_summary, summary_updated_at";

// ---- goals ----

pub async fn create_goal(conn: &Connection, rec: &ProjectGoalRecord) -> Result<()> {
    conn.execute(
        "INSERT INTO project_goals (id, title, detail_md, status, sort_key, created_at, updated_at, current_summary, summary_updated_at) VALUES (?,?,?,?,?,?,?,?,?)",
        params![
            rec.id.as_str(),
            rec.title.as_str(),
            rec.detail_md.as_deref(),
            rec.status.as_str(),
            rec.sort,
            rec.created_at,
            rec.updated_at,
            rec.summary.current_summary.as_str(),
            rec.summary.summary_updated_at
        ],
    )
    .await
    .context("insert project goal")?;
    people::patch(conn, "project", &rec.id, &rec.people.replacement()).await?;
    summaries::append(
        conn,
        SummaryScope::Project,
        &rec.id,
        &rec.summary.current_summary,
        rec.summary.summary_updated_at.unwrap_or(rec.created_at),
        None,
    )
    .await?;
    Ok(())
}

/// Dynamic `SET` from the patch's `Some` fields; always stamps
/// `updated_at = now_ms`. Returns `false` when the id does not exist.
pub async fn patch_goal(
    conn: &Connection,
    id: &str,
    patch: &ProjectGoalPatch,
    now_ms: i64,
) -> Result<bool> {
    let mut sets: Vec<&'static str> = Vec::new();
    let mut vals: Vec<Value> = Vec::new();
    if let Some(v) = patch.title.as_deref() {
        sets.push("title = ?");
        vals.push(v.into());
    }
    if let Some(v) = patch.detail_md.as_deref() {
        sets.push("detail_md = ?");
        vals.push(v.into());
    }
    if let Some(v) = patch.status {
        sets.push("status = ?");
        vals.push(v.as_str().into());
    }
    if let Some(v) = patch.sort {
        sets.push("sort_key = ?");
        vals.push(v.into());
    }
    if let Some(text) = &patch.current_summary {
        sets.extend(["current_summary = ?", "summary_updated_at = ?"]);
        vals.extend([text.trim().into(), now_ms.into()]);
    }
    sets.push("updated_at = ?");
    vals.push(now_ms.into());
    let sql = format!("UPDATE project_goals SET {} WHERE id = ?", sets.join(", "));
    vals.push(id.into());
    let n = conn
        .execute(&sql, vals)
        .await
        .context("patch project goal")?;
    if n > 0 {
        people::patch(conn, "project", id, &patch.people).await?;
        if let Some(text) = &patch.current_summary {
            summaries::append(conn, SummaryScope::Project, id, text, now_ms, None).await?;
        }
    }
    Ok(n > 0)
}

/// Delete the goal, preserving its initiatives, TODOs and runs.
pub async fn delete_goal(conn: &Connection, id: &str) -> Result<bool> {
    super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        if !exists(conn, "SELECT 1 FROM project_goals WHERE id = ?1", id).await? {
            return Ok(false);
        }
        let previous_tags = tags::list(conn).await?;
        conn.execute(
            "UPDATE project_initiatives SET goal_id = NULL WHERE goal_id = ?1",
            params![id],
        )
        .await?;
        conn.execute("DELETE FROM project_goals WHERE id = ?1", params![id])
            .await?;
        summaries::remove(conn, SummaryScope::Project, id).await?;
        conn.execute(
            "DELETE FROM project_tags WHERE scope_type='project' AND scope_id=?",
            params![id],
        )
        .await?;
        tags::reconcile(conn, &previous_tags, None).await?;
        Ok(true)
    })
    .await
}

/// Ordered by `sort_key` then `created_at`.
pub async fn list_goals(conn: &Connection) -> Result<Vec<ProjectGoalRecord>> {
    let mut people = people::list(conn, "project", None).await?;
    let mut stmt = conn
        .prepare_cached(&format!(
            "SELECT {GOAL_COLS} FROM project_goals ORDER BY sort_key, created_at"
        ))
        .await?;
    let mut rows = stmt.query(()).await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        let mut record = row_to_goal(&r)?;
        record.people = people.remove(&record.id).unwrap_or_default();
        out.push(record);
    }
    Ok(out)
}

fn row_to_goal(r: &turso::Row) -> Result<ProjectGoalRecord> {
    Ok(ProjectGoalRecord {
        summary: crate::project::summary::ProjectSummary {
            current_summary: r.get(7)?,
            summary_updated_at: r.get(8)?,
        },
        people: Default::default(),
        id: r.get(0)?,
        title: r.get(1)?,
        detail_md: r.get(2)?,
        // An unparseable status is corruption: propagate instead of coercing.
        status: ProjectGoalStatus::parse(&r.get::<String>(3)?).context("project_goals.status")?,
        sort: r.get(4)?,
        created_at: r.get(5)?,
        updated_at: r.get(6)?,
    })
}

// ---- initiatives ----

pub async fn create_initiative(conn: &Connection, rec: &ProjectInitiativeRecord) -> Result<()> {
    conn.execute(
        "INSERT INTO project_initiatives (id, goal_id, title, detail_md, sort_key, created_at, updated_at, current_summary, summary_updated_at) VALUES (?,?,?,?,?,?,?,?,?)",
        params![
            rec.id.as_str(),
            rec.goal_id.as_deref(),
            rec.title.as_str(),
            rec.detail_md.as_deref(),
            rec.sort,
            rec.created_at,
            rec.updated_at,
            rec.summary.current_summary.as_str(),
            rec.summary.summary_updated_at
        ],
    )
    .await
    .context("insert project initiative")?;
    people::patch(conn, "initiative", &rec.id, &rec.people.replacement()).await?;
    summaries::append(
        conn,
        SummaryScope::Initiative,
        &rec.id,
        &rec.summary.current_summary,
        rec.summary.summary_updated_at.unwrap_or(rec.created_at),
        None,
    )
    .await?;
    Ok(())
}

pub async fn patch_initiative(
    conn: &Connection,
    id: &str,
    patch: &ProjectInitiativePatch,
    now_ms: i64,
) -> Result<bool> {
    let mut sets: Vec<&'static str> = Vec::new();
    let mut vals: Vec<Value> = Vec::new();
    if let Some(v) = &patch.goal_id {
        sets.push("goal_id = ?");
        vals.push(v.as_deref().map(Value::from).unwrap_or(Value::Null));
    }
    if let Some(v) = patch.title.as_deref() {
        sets.push("title = ?");
        vals.push(v.into());
    }
    if let Some(v) = patch.detail_md.as_deref() {
        sets.push("detail_md = ?");
        vals.push(v.into());
    }
    if let Some(v) = patch.sort {
        sets.push("sort_key = ?");
        vals.push(v.into());
    }
    if let Some(text) = &patch.current_summary {
        sets.extend(["current_summary = ?", "summary_updated_at = ?"]);
        vals.extend([text.trim().into(), now_ms.into()]);
    }
    sets.push("updated_at = ?");
    vals.push(now_ms.into());
    let sql = format!(
        "UPDATE project_initiatives SET {} WHERE id = ?",
        sets.join(", ")
    );
    vals.push(id.into());
    let n = conn
        .execute(&sql, vals)
        .await
        .context("patch project initiative")?;
    if n > 0 {
        people::patch(conn, "initiative", id, &patch.people).await?;
        if let Some(text) = &patch.current_summary {
            summaries::append(conn, SummaryScope::Initiative, id, text, now_ms, None).await?;
        }
    }
    Ok(n > 0)
}

/// Only empty initiatives can be deleted; associations must be changed explicitly.
pub async fn delete_initiative(conn: &Connection, id: &str) -> Result<bool> {
    super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        let mut stmt = conn
            .prepare_cached("SELECT 1 FROM project_initiatives WHERE id = ?1")
            .await?;
        let mut rows = stmt.query(params![id]).await?;
        let found = rows.next().await?.is_some();
        drop(rows);
        if !found {
            return Ok(false);
        }
        if exists(
            conn,
            "SELECT 1 FROM project_todos WHERE initiative_id = ?1",
            id,
        )
        .await?
        {
            return Err(crate::project::InitiativeNotEmpty.into());
        }
        conn.execute("DELETE FROM project_initiatives WHERE id = ?1", params![id])
            .await?;
        summaries::remove(conn, SummaryScope::Initiative, id).await?;
        let previous = tags::list(conn).await?;
        conn.execute(
            "DELETE FROM project_tags WHERE scope_type='initiative' AND scope_id=?",
            params![id],
        )
        .await?;
        tags::reconcile(conn, &previous, None).await?;
        Ok(true)
    })
    .await
}

/// `goal_id == None` lists across all goals; ordered by `sort_key` then
/// `created_at`.
pub async fn list_initiatives(
    conn: &Connection,
    goal_id: Option<&str>,
) -> Result<Vec<ProjectInitiativeRecord>> {
    let mut people = people::list(conn, "initiative", None).await?;
    let mut sql = format!("SELECT {INITIATIVE_COLS} FROM project_initiatives ");
    if goal_id.is_some() {
        sql.push_str(" WHERE goal_id = ?");
    }
    sql.push_str(" ORDER BY sort_key, created_at, id");
    let mut stmt = conn.prepare_cached(&sql).await?;
    let mut rows = match goal_id {
        Some(g) => stmt.query(params![g]).await?,
        None => stmt.query(()).await?,
    };
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        let mut record = row_to_initiative(&r)?;
        record.people = people.remove(&record.id).unwrap_or_default();
        out.push(record);
    }
    Ok(out)
}

fn row_to_initiative(r: &turso::Row) -> Result<ProjectInitiativeRecord> {
    Ok(ProjectInitiativeRecord {
        summary: crate::project::summary::ProjectSummary {
            current_summary: r.get(7)?,
            summary_updated_at: r.get(8)?,
        },
        people: Default::default(),
        id: r.get(0)?,
        goal_id: r.get(1)?,
        title: r.get(2)?,
        detail_md: r.get(3)?,
        sort: r.get(4)?,
        created_at: r.get(5)?,
        updated_at: r.get(6)?,
    })
}

async fn exists(conn: &Connection, sql: &str, id: &str) -> Result<bool> {
    let mut stmt = conn.prepare_cached(sql).await?;
    let mut rows = stmt.query(params![id]).await?;
    Ok(rows.next().await?.is_some())
}

include!("project/implementation/mod.rs");
