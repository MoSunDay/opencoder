//! Project-module persistence trait — the seam that lets the project tables
//! (goals / milestones / todos / runs) live in libsql today and in an
//! external MySQL / StarRocks tomorrow without touching upper layers.
//!
//! Upper-layer code depends on `Arc<dyn ProjectStore>`; the concrete libsql
//! implementation lives in `libsql_store::project` (+ `project_runs`).

use anyhow::Result;
use async_trait::async_trait;

use crate::project_types::{
    ProjectGoalPatch, ProjectGoalRecord, ProjectMilestonePatch, ProjectMilestoneRecord,
    ProjectTodoPatch, ProjectTodoRecord, ProjectTodoRunPatch, ProjectTodoRunRecord,
};

/// CRUD for the project module's four tables.
///
/// Conventions shared by all implementations:
/// - `patch_*` returns `false` when the id does not exist (0 rows affected).
/// - `delete_*` returns `false` when the id does not exist.
/// - Deletes cascade inside one transaction, explicitly (not via backend FK
///   tricks), so every backend behaves identically.
/// - Status/kind strings round-trip exactly; an unrecognized status on read is
///   corruption and propagates as an error.
#[async_trait]
pub trait ProjectStore: Send + Sync {
    /// Backend identifier for diagnostics ("libsql", "mysql", ...).
    fn project_backend_name(&self) -> &'static str;

    // ---- goals ----

    async fn create_goal(&self, rec: &ProjectGoalRecord) -> Result<()>;
    /// `false` = id not found. Always stamps `updated_at = now_ms`.
    async fn patch_goal(&self, id: &str, patch: &ProjectGoalPatch, now_ms: i64) -> Result<bool>;
    /// Transactional cascade: runs → todos → milestones → goal.
    async fn delete_goal(&self, id: &str) -> Result<bool>;
    /// Ordered by `sort` then `created_at`.
    async fn list_goals(&self) -> Result<Vec<ProjectGoalRecord>>;

    // ---- milestones ----

    async fn create_milestone(&self, rec: &ProjectMilestoneRecord) -> Result<()>;
    async fn patch_milestone(
        &self,
        id: &str,
        patch: &ProjectMilestonePatch,
        now_ms: i64,
    ) -> Result<bool>;
    /// Transactional cascade: the runs and todos of THIS milestone, then the
    /// milestone itself. Decision: its todos are deleted, NOT re-parented to
    /// the backlog — deleting a milestone is a destructive, user-confirmed
    /// action, and silently resurrecting its todos as backlog items would
    /// resurrect stale work the user meant to remove.
    async fn delete_milestone(&self, id: &str) -> Result<bool>;
    /// `goal_id == None` lists across all goals; ordered by `sort` then
    /// `created_at`.
    async fn list_milestones(&self, goal_id: Option<&str>) -> Result<Vec<ProjectMilestoneRecord>>;

    // ---- todos ----

    async fn create_todo(&self, rec: &ProjectTodoRecord) -> Result<()>;
    async fn patch_todo(&self, id: &str, patch: &ProjectTodoPatch, now_ms: i64) -> Result<bool>;
    /// Transactional cascade: the todo's runs, then the todo.
    async fn delete_todo(&self, id: &str) -> Result<bool>;
    async fn get_todo(&self, id: &str) -> Result<Option<ProjectTodoRecord>>;
    /// `milestone_id == None` lists ALL todos (backlog included); ordered by
    /// `created_at`.
    async fn list_todos(&self, milestone_id: Option<&str>) -> Result<Vec<ProjectTodoRecord>>;

    // ---- todo runs ----

    async fn create_todo_run(&self, rec: &ProjectTodoRunRecord) -> Result<()>;
    async fn patch_todo_run(
        &self,
        id: &str,
        patch: &ProjectTodoRunPatch,
        now_ms: i64,
    ) -> Result<bool>;
    async fn get_todo_run(&self, id: &str) -> Result<Option<ProjectTodoRunRecord>>;
    /// Newest version first.
    async fn list_todo_runs(&self, todo_id: &str) -> Result<Vec<ProjectTodoRunRecord>>;
    /// `COALESCE(MAX(version), 0) + 1` for the todo — 1 for a fresh todo.
    async fn next_todo_version(&self, todo_id: &str) -> Result<i64>;
}
