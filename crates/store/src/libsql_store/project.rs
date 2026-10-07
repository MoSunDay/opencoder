//! Project-module persistence — goals & initiatives CRUD (libsql).
//!
//! Free functions over a raw `Connection`, mirroring sibling submodules;
//! multi-statement deletes run via [`super::tx::run_tx`] (`BEGIN IMMEDIATE`)
//! and cascade explicitly (not via FK tricks) so every backend behaves the
//! same. Todo/run CRUD lives in [`super::project_runs`]; the `ProjectStore`
//! impl at the bottom delegates to both.

use anyhow::{Context, Result};
use libsql::{params, Connection, Value};

use super::LibsqlStore;
mod tags;
use crate::project::ProjectStore;
use crate::project_types::{
    ProjectGoalPatch, ProjectGoalRecord, ProjectGoalStatus, ProjectInitiativePatch,
    ProjectInitiativeRecord, ProjectInitiativeStatus, ProjectTodoPatch, ProjectTodoRecord,
    ProjectTodoRunPatch, ProjectTodoRunRecord, ProjectTodoRunStatus, ProjectTodoStatus,
};

const GOAL_COLS: &str = "id, title, detail_md, status, sort_key, created_at, updated_at";
const INITIATIVE_COLS: &str =
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

// ---- initiatives ----

pub async fn create_initiative(conn: &Connection, rec: &ProjectInitiativeRecord) -> Result<()> {
    conn.execute(
        "INSERT INTO project_initiatives (id, goal_id, title, detail_md, status, sort_key, created_at, updated_at) VALUES (?,?,?,?,?,?,?,?)",
        params![
            rec.id.as_str(),
            rec.goal_id.as_deref(),
            rec.title.as_str(),
            rec.detail_md.as_deref(),
            rec.status.as_str(),
            rec.sort,
            rec.created_at,
            rec.updated_at
        ],
    )
    .await
    .context("insert project initiative")?;
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
        "UPDATE project_initiatives SET {} WHERE id = ?",
        sets.join(", ")
    );
    vals.push(id.into());
    let n = conn
        .execute(&sql, vals)
        .await
        .context("patch project initiative")?;
    Ok(n > 0)
}

/// Only empty initiatives can be deleted; associations must be changed explicitly.
pub async fn delete_initiative(conn: &Connection, id: &str) -> Result<bool> {
    super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        let stmt = conn
            .prepare("SELECT 1 FROM project_initiatives WHERE id = ?1")
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
    let mut sql = format!("SELECT {INITIATIVE_COLS} FROM project_initiatives ");
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
        out.push(row_to_initiative(&r)?);
    }
    Ok(out)
}

fn row_to_initiative(r: &libsql::Row) -> Result<ProjectInitiativeRecord> {
    Ok(ProjectInitiativeRecord {
        id: r.get(0)?,
        goal_id: r.get(1)?,
        title: r.get(2)?,
        detail_md: r.get(3)?,
        status: ProjectInitiativeStatus::parse(&r.get::<String>(4)?)
            .context("project_initiatives.status")?,
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

    async fn list_tags(&self) -> Result<Vec<crate::project::ProjectTag>> {
        let _guard = self.db_lock.lock().await;
        tags::list(&self.conn().await?).await
    }
    async fn list_todo_tags(&self) -> Result<Vec<crate::project::ProjectTodoTag>> {
        let _guard = self.db_lock.lock().await;
        tags::links(&self.conn().await?).await
    }
    async fn write_tag(&self, tag: &crate::project::ProjectTag) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        tags::write(&self.conn().await?, tag).await
    }
    async fn delete_tag(&self, id: &str) -> Result<bool> {
        let _guard = self.db_lock.lock().await;
        tags::delete(&self.conn().await?, id).await
    }
    async fn create_todo_tagged(&self, rec: &ProjectTodoRecord, ids: &[String]) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        tags::create_todo(&self.conn().await?, rec, ids).await
    }
    async fn patch_todo_tagged(
        &self,
        id: &str,
        patch: &ProjectTodoPatch,
        ids: Option<&[String]>,
        now: i64,
    ) -> Result<bool> {
        let _guard = self.db_lock.lock().await;
        tags::patch_todo(&self.conn().await?, id, patch, ids, now).await
    }

    async fn list_todo_assignments(
        &self,
        todo_id: &str,
    ) -> Result<Vec<crate::project::ProjectAssignment>> {
        let _guard = self.db_lock.lock().await;
        super::project_links::list(&self.conn().await?, todo_id).await
    }

    async fn latest_todo_assignments(&self) -> Result<Vec<crate::project::ProjectAssignment>> {
        let _guard = self.db_lock.lock().await;
        super::project_links::latest(&self.conn().await?).await
    }

    async fn link_todo_execution(
        &self,
        assignment: &crate::project::ProjectAssignment,
    ) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        super::project_links::link(&self.conn().await?, assignment).await
    }

    async fn unlink_todo_execution(&self, todo_id: &str, execution_id: &str) -> Result<bool> {
        let _guard = self.db_lock.lock().await;
        super::project_links::unlink(&self.conn().await?, todo_id, execution_id).await
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

    async fn create_initiative(&self, rec: &ProjectInitiativeRecord) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        create_initiative(&self.conn().await?, rec).await
    }
    async fn patch_initiative(
        &self,
        id: &str,
        patch: &ProjectInitiativePatch,
        now_ms: i64,
    ) -> Result<bool> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        super::tx::run_tx(&conn, "BEGIN IMMEDIATE", || async {
            let changed = patch_initiative(&conn, id, patch, now_ms).await?;
            if changed && patch.goal_id.is_some() {
                tags::reconcile(&conn, &[], None).await?;
            }
            Ok(changed)
        })
        .await
    }
    async fn delete_initiative(&self, id: &str) -> Result<bool> {
        let _guard = self.db_lock.lock().await;
        delete_initiative(&self.conn().await?, id).await
    }
    async fn list_initiatives(
        &self,
        goal_id: Option<&str>,
    ) -> Result<Vec<ProjectInitiativeRecord>> {
        let _guard = self.db_lock.lock().await;
        list_initiatives(&self.conn().await?, goal_id).await
    }

    async fn create_todo(&self, rec: &ProjectTodoRecord) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        tags::create_todo(&conn, rec, &[]).await
    }
    async fn patch_todo(&self, id: &str, patch: &ProjectTodoPatch, now_ms: i64) -> Result<bool> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        tags::patch_todo(&conn, id, patch, None, now_ms).await
    }
    async fn claim_todo_running(&self, id: &str, now_ms: i64) -> Result<bool> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        super::project_runs::claim_todo_running(&conn, id, now_ms).await
    }
    async fn claim_todo_running_with_run(
        &self,
        rec: &ProjectTodoRunRecord,
        now_ms: i64,
    ) -> Result<bool> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        super::project_runs::claim_todo_running_with_run(&conn, rec, now_ms).await
    }
    async fn patch_todo_when(
        &self,
        id: &str,
        when: ProjectTodoStatus,
        patch: &ProjectTodoPatch,
        now_ms: i64,
    ) -> Result<bool> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        super::project_runs::patch_todo_when(&conn, id, when, patch, now_ms).await
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
    async fn reorder_todos(
        &self,
        initiative_id: Option<&str>,
        board_status: &str,
        ids: &[String],
        now_ms: i64,
    ) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        super::project_runs::reorder_todos(&conn, initiative_id, board_status, ids, now_ms).await
    }
    async fn get_todo_summary(&self, id: &str) -> Result<Option<crate::ProjectTodoSummary>> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        super::project_runs::get_todo_summary(&conn, id).await
    }
    async fn list_todos(&self, initiative_id: Option<&str>) -> Result<Vec<ProjectTodoRecord>> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        super::project_runs::list_todos(&conn, initiative_id).await
    }

    async fn create_todo_run(&self, rec: &ProjectTodoRunRecord) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        super::project_runs::create_todo_run(&conn, rec).await
    }
    async fn finish_todo_run(
        &self,
        id: &str,
        patch: &ProjectTodoRunPatch,
        now_ms: i64,
    ) -> Result<bool> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        super::project_runs::finish_todo_run(&conn, id, patch, now_ms).await
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
    async fn patch_todo_run_when(
        &self,
        id: &str,
        when: ProjectTodoRunStatus,
        patch: &ProjectTodoRunPatch,
        now_ms: i64,
    ) -> Result<bool> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        super::project_runs::patch_todo_run_when(&conn, id, when, patch, now_ms).await
    }
    async fn get_todo_run(&self, id: &str) -> Result<Option<ProjectTodoRunRecord>> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        super::project_runs::get_todo_run(&conn, id).await
    }
    async fn get_todo_run_summary(&self, id: &str) -> Result<Option<crate::ProjectTodoRunSummary>> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        super::project_runs::get_todo_run_summary(&conn, id).await
    }
    async fn list_todo_runs(&self, todo_id: &str) -> Result<Vec<ProjectTodoRunRecord>> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        super::project_runs::list_todo_runs(&conn, todo_id).await
    }
    async fn list_todo_runs_page(
        &self,
        todo_id: &str,
        before_version: Option<i64>,
        limit: u32,
    ) -> Result<crate::ProjectTodoRunPage> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        super::project_runs::list_todo_runs_page(&conn, todo_id, before_version, limit).await
    }
    async fn project_text_chunk(
        &self,
        record_kind: &str,
        owner_id: &str,
        id: &str,
        field: &str,
        offset: u64,
        max_bytes: usize,
    ) -> Result<Option<crate::PayloadChunkRecord>> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        super::project_runs::project_text_chunk(
            &conn,
            record_kind,
            owner_id,
            id,
            field,
            offset,
            max_bytes,
        )
        .await
    }
    async fn list_running_todo_runs(&self) -> Result<Vec<ProjectTodoRunRecord>> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        super::project_runs::list_running_todo_runs(&conn).await
    }
    async fn next_todo_version(&self, todo_id: &str) -> Result<i64> {
        let _guard = self.db_lock.lock().await;
        let conn = self.conn().await?;
        super::project_runs::next_todo_version(&conn, todo_id).await
    }
}
