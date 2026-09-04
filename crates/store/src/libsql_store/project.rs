//! Project-module persistence — goals & milestones CRUD (libsql).
//!
//! Free functions over a raw `Connection`, mirroring sibling submodules;
//! multi-statement deletes run via [`super::tx::run_tx`] (`BEGIN IMMEDIATE`)
//! and cascade explicitly (not via FK tricks) so every backend behaves the
//! same. Todo/run CRUD lives in [`super::project_runs`]; the `ProjectStore`
//! impl at the bottom delegates to both.

use anyhow::{Context, Result};
use libsql::{params, Connection, Value};

use super::LibsqlStore;
use crate::project::ProjectStore;
use crate::project_types::{
    ProjectGoalPatch, ProjectGoalRecord, ProjectGoalStatus, ProjectMilestonePatch,
    ProjectMilestoneRecord, ProjectMilestoneStatus, ProjectTodoPatch, ProjectTodoRecord,
    ProjectTodoRunPatch, ProjectTodoRunRecord,
};

const GOAL_COLS: &str = "id, title, detail_md, status, sort_key, created_at, updated_at";
const MILESTONE_COLS: &str =
    "id, goal_id, title, detail_md, status, sort_key, created_at, updated_at";

// ---- goals ----

pub async fn create_goal(conn: &Connection, rec: &ProjectGoalRecord) -> Result<()> {
    conn.execute(
        "INSERT INTO project_goals (id, title, detail_md, status, sort_key, created_at, updated_at) VALUES (?,?,?,?,?,?,?)",
        params![
            rec.id.as_str(),
            rec.title.as_str(),
            rec.detail_md.as_deref(),
            rec.status.as_str(),
            rec.sort,
            rec.created_at,
            rec.updated_at
        ],
    )
    .await
    .context("insert project goal")?;
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
    sets.push("updated_at = ?");
    vals.push(now_ms.into());
    let sql = format!("UPDATE project_goals SET {} WHERE id = ?", sets.join(", "));
    vals.push(id.into());
    let n = conn
        .execute(&sql, vals)
        .await
        .context("patch project goal")?;
    Ok(n > 0)
}

/// Tx cascade: runs (via todos of the goal's milestones) → todos →
/// milestones → goal. `false` when the goal id does not exist.
pub async fn delete_goal(conn: &Connection, id: &str) -> Result<bool> {
    super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        if !exists(conn, "SELECT 1 FROM project_goals WHERE id = ?1", id).await? {
            return Ok(false);
        }
        conn.execute(
            "DELETE FROM project_todo_runs WHERE todo_id IN (
               SELECT t.id FROM project_todos t
               JOIN project_milestones m ON m.id = t.milestone_id
               WHERE m.goal_id = ?1)",
            params![id],
        )
        .await
        .context("cascade delete goal runs")?;
        conn.execute(
            "DELETE FROM project_todos WHERE milestone_id IN \
             (SELECT id FROM project_milestones WHERE goal_id = ?1)",
            params![id],
        )
        .await
        .context("cascade delete goal todos")?;
        conn.execute(
            "DELETE FROM project_milestones WHERE goal_id = ?1",
            params![id],
        )
        .await
        .context("cascade delete goal milestones")?;
        conn.execute("DELETE FROM project_goals WHERE id = ?1", params![id])
            .await
            .context("delete project goal")?;
        Ok(true)
    })
    .await
}

/// Ordered by `sort_key` then `created_at`.
pub async fn list_goals(conn: &Connection) -> Result<Vec<ProjectGoalRecord>> {
    let stmt = conn
        .prepare(&format!(
            "SELECT {GOAL_COLS} FROM project_goals ORDER BY sort_key, created_at"
        ))
        .await?;
    let mut rows = stmt.query(()).await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        out.push(row_to_goal(&r)?);
    }
    Ok(out)
}

fn row_to_goal(r: &libsql::Row) -> Result<ProjectGoalRecord> {
    Ok(ProjectGoalRecord {
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

// ---- milestones ----

pub async fn create_milestone(conn: &Connection, rec: &ProjectMilestoneRecord) -> Result<()> {
    conn.execute(
        "INSERT INTO project_milestones (id, goal_id, title, detail_md, status, sort_key, created_at, updated_at) VALUES (?,?,?,?,?,?,?,?)",
        params![
            rec.id.as_str(),
            rec.goal_id.as_str(),
            rec.title.as_str(),
            rec.detail_md.as_deref(),
            rec.status.as_str(),
            rec.sort,
            rec.created_at,
            rec.updated_at
        ],
    )
    .await
    .context("insert project milestone")?;
    Ok(())
}

pub async fn patch_milestone(
    conn: &Connection,
    id: &str,
    patch: &ProjectMilestonePatch,
    now_ms: i64,
) -> Result<bool> {
    let mut sets: Vec<&'static str> = Vec::new();
    let mut vals: Vec<Value> = Vec::new();
    if let Some(v) = patch.goal_id.as_deref() {
        sets.push("goal_id = ?");
        vals.push(v.into());
    }
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
    sets.push("updated_at = ?");
    vals.push(now_ms.into());
    let sql = format!(
        "UPDATE project_milestones SET {} WHERE id = ?",
        sets.join(", ")
    );
    vals.push(id.into());
    let n = conn
        .execute(&sql, vals)
        .await
        .context("patch project milestone")?;
    Ok(n > 0)
}

/// Tx cascade: the runs of this milestone's todos → the todos themselves →
/// the milestone. Todos are deleted, NOT re-parented to the backlog (see the
/// trait docs). `false` when the milestone id does not exist.
pub async fn delete_milestone(conn: &Connection, id: &str) -> Result<bool> {
    super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        if !exists(conn, "SELECT 1 FROM project_milestones WHERE id = ?1", id).await? {
            return Ok(false);
        }
        conn.execute(
            "DELETE FROM project_todo_runs WHERE todo_id IN \
             (SELECT id FROM project_todos WHERE milestone_id = ?1)",
            params![id],
        )
        .await
        .context("cascade delete milestone runs")?;
        conn.execute(
            "DELETE FROM project_todos WHERE milestone_id = ?1",
            params![id],
        )
        .await
        .context("cascade delete milestone todos")?;
        conn.execute("DELETE FROM project_milestones WHERE id = ?1", params![id])
            .await
            .context("delete project milestone")?;
        Ok(true)
    })
    .await
}

/// `goal_id == None` lists across all goals; ordered by `sort_key` then
/// `created_at`.
pub async fn list_milestones(
    conn: &Connection,
    goal_id: Option<&str>,
) -> Result<Vec<ProjectMilestoneRecord>> {
    let mut sql = format!("SELECT {MILESTONE_COLS} FROM project_milestones");
    if goal_id.is_some() {
        sql.push_str(" WHERE goal_id = ?");
    }
    sql.push_str(" ORDER BY sort_key, created_at");
    let stmt = conn.prepare(&sql).await?;
    let mut rows = match goal_id {
        Some(g) => stmt.query(params![g]).await?,
        None => stmt.query(()).await?,
    };
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        out.push(row_to_milestone(&r)?);
    }
    Ok(out)
}

fn row_to_milestone(r: &libsql::Row) -> Result<ProjectMilestoneRecord> {
    Ok(ProjectMilestoneRecord {
        id: r.get(0)?,
        goal_id: r.get(1)?,
        title: r.get(2)?,
        detail_md: r.get(3)?,
        status: ProjectMilestoneStatus::parse(&r.get::<String>(4)?)
            .context("project_milestones.status")?,
        sort: r.get(5)?,
        created_at: r.get(6)?,
        updated_at: r.get(7)?,
    })
}

async fn exists(conn: &Connection, sql: &str, id: &str) -> Result<bool> {
    let stmt = conn.prepare(sql).await?;
    let mut rows = stmt.query(params![id]).await?;
    Ok(rows.next().await?.is_some())
}

/// `ProjectStore` for the embedded libsql backend. Every method takes the
/// store-wide `db_lock` (serializes SQLite FFI — see `LibsqlStore` docs) and
/// delegates to the free functions above / in `project_runs`.
#[async_trait::async_trait]
impl ProjectStore for LibsqlStore {
    fn project_backend_name(&self) -> &'static str {
        "libsql"
    }

    async fn create_goal(&self, rec: &ProjectGoalRecord) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        create_goal(&conn, rec).await
    }
    async fn patch_goal(&self, id: &str, patch: &ProjectGoalPatch, now_ms: i64) -> Result<bool> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        patch_goal(&conn, id, patch, now_ms).await
    }
    async fn delete_goal(&self, id: &str) -> Result<bool> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        delete_goal(&conn, id).await
    }
    async fn list_goals(&self) -> Result<Vec<ProjectGoalRecord>> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        list_goals(&conn).await
    }

    async fn create_milestone(&self, rec: &ProjectMilestoneRecord) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        create_milestone(&conn, rec).await
    }
    async fn patch_milestone(
        &self,
        id: &str,
        patch: &ProjectMilestonePatch,
        now_ms: i64,
    ) -> Result<bool> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        patch_milestone(&conn, id, patch, now_ms).await
    }
    async fn delete_milestone(&self, id: &str) -> Result<bool> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        delete_milestone(&conn, id).await
    }
    async fn list_milestones(&self, goal_id: Option<&str>) -> Result<Vec<ProjectMilestoneRecord>> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        list_milestones(&conn, goal_id).await
    }

    async fn create_todo(&self, rec: &ProjectTodoRecord) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        super::project_runs::create_todo(&conn, rec).await
    }
    async fn patch_todo(&self, id: &str, patch: &ProjectTodoPatch, now_ms: i64) -> Result<bool> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        super::project_runs::patch_todo(&conn, id, patch, now_ms).await
    }
    async fn delete_todo(&self, id: &str) -> Result<bool> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        super::project_runs::delete_todo(&conn, id).await
    }
    async fn get_todo(&self, id: &str) -> Result<Option<ProjectTodoRecord>> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        super::project_runs::get_todo(&conn, id).await
    }
    async fn list_todos(&self, milestone_id: Option<&str>) -> Result<Vec<ProjectTodoRecord>> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        super::project_runs::list_todos(&conn, milestone_id).await
    }

    async fn create_todo_run(&self, rec: &ProjectTodoRunRecord) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        super::project_runs::create_todo_run(&conn, rec).await
    }
    async fn patch_todo_run(
        &self,
        id: &str,
        patch: &ProjectTodoRunPatch,
        now_ms: i64,
    ) -> Result<bool> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        super::project_runs::patch_todo_run(&conn, id, patch, now_ms).await
    }
    async fn get_todo_run(&self, id: &str) -> Result<Option<ProjectTodoRunRecord>> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        super::project_runs::get_todo_run(&conn, id).await
    }
    async fn list_todo_runs(&self, todo_id: &str) -> Result<Vec<ProjectTodoRunRecord>> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        super::project_runs::list_todo_runs(&conn, todo_id).await
    }
    async fn next_todo_version(&self, todo_id: &str) -> Result<i64> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        super::project_runs::next_todo_version(&conn, todo_id).await
    }
}
