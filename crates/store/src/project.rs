//! Project-module persistence trait — the seam that lets the project tables
//! (goals / initiatives / todos / runs) live in libsql today and in an
//! external MySQL / StarRocks tomorrow without touching upper layers.
//!
//! Upper-layer code depends on `Arc<dyn ProjectStore>`; the concrete libsql
//! implementation lives in `libsql_store::project` (+ `project_runs`).

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub mod overview;
pub mod tags;
pub use tags::{ProjectTag, ProjectTodoTag};

/// A project owns references, never a copy of an execution's state or output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectAssignment {
    pub todo_id: String,
    pub execution_id: String,
    pub capability_id: Option<String>,
    pub kind: String,
    pub name: String,
    pub created_at: i64,
}

#[derive(Debug)]
pub struct InitiativeNotEmpty;

impl std::fmt::Display for InitiativeNotEmpty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("initiative contains TODOs; move or unlink them before deleting")
    }
}

impl std::error::Error for InitiativeNotEmpty {}

use crate::project_types::{
    ProjectGoalPatch, ProjectGoalRecord, ProjectInitiativePatch, ProjectInitiativeRecord,
    ProjectTodoPatch, ProjectTodoRecord, ProjectTodoRunPatch, ProjectTodoRunRecord,
    ProjectTodoRunStatus, ProjectTodoRunSummary, ProjectTodoStatus, ProjectTodoSummary,
};

/// CRUD for the project catalog and execution records.
///
/// Conventions shared by all implementations:
/// - `patch_*` returns `false` when the id does not exist (0 rows affected).
/// - Every `patch_*` builds its `SET` clause from the patch's `Some` fields
///   only; a patch with every field `None` is a caller bug (it would produce
///   an invalid empty `SET`).
/// - `delete_*` returns `false` when the id does not exist.
/// - Goal deletion detaches initiatives; nonempty initiatives reject deletion.
///   TODO deletion also removes its runs. libsql/MySQL use transactions;
///   StarRocks performs the statements sequentially.
/// - Status/kind strings round-trip exactly; an unrecognized status on read is
///   corruption and propagates as an error.
#[async_trait]
// async_trait annotates futures that are already must-use on Rust 1.99.
#[allow(clippy::double_must_use)]
pub trait ProjectStore: Send + Sync {
    /// Backend identifier for diagnostics ("libsql", "mysql", ...).
    fn project_backend_name(&self) -> &'static str;

    async fn list_tags(&self) -> Result<Vec<ProjectTag>> {
        Ok(vec![])
    }
    async fn list_todo_tags(&self) -> Result<Vec<ProjectTodoTag>> {
        Ok(vec![])
    }
    async fn write_tag(&self, _tag: &ProjectTag) -> Result<()> {
        anyhow::bail!("project tags are unsupported by this store")
    }
    async fn delete_tag(&self, _id: &str) -> Result<bool> {
        anyhow::bail!("project tags are unsupported by this store")
    }
    async fn create_todo_tagged(&self, rec: &ProjectTodoRecord, tags: &[String]) -> Result<()> {
        anyhow::ensure!(
            tags.is_empty(),
            "project tags are unsupported by this store"
        );
        self.create_todo(rec).await
    }
    async fn patch_todo_tagged(
        &self,
        id: &str,
        patch: &ProjectTodoPatch,
        tags: Option<&[String]>,
        now: i64,
    ) -> Result<bool> {
        anyhow::ensure!(
            tags.is_none_or(|tags| tags.is_empty()),
            "project tags are unsupported by this store"
        );
        self.patch_todo(id, patch, now).await
    }

    // ---- goals ----

    async fn create_goal(&self, rec: &ProjectGoalRecord) -> Result<()>;
    /// `false` = id not found. Always stamps `updated_at = now_ms`.
    async fn patch_goal(&self, id: &str, patch: &ProjectGoalPatch, now_ms: i64) -> Result<bool>;
    /// Detach initiatives and initiatives; preserve their TODOs.
    async fn delete_goal(&self, id: &str) -> Result<bool>;
    /// Ordered by `sort` then `created_at`.
    async fn list_goals(&self) -> Result<Vec<ProjectGoalRecord>>;

    // ---- initiatives ----

    async fn create_initiative(&self, rec: &ProjectInitiativeRecord) -> Result<()>;
    async fn patch_initiative(
        &self,
        id: &str,
        patch: &ProjectInitiativePatch,
        now_ms: i64,
    ) -> Result<bool>;
    async fn delete_initiative(&self, id: &str) -> Result<bool>;
    async fn list_initiatives(&self, goal_id: Option<&str>)
        -> Result<Vec<ProjectInitiativeRecord>>;

    // ---- todos ----

    async fn create_todo(&self, rec: &ProjectTodoRecord) -> Result<()>;
    async fn patch_todo(&self, id: &str, patch: &ProjectTodoPatch, now_ms: i64) -> Result<bool>;
    async fn reorder_todos(
        &self,
        _initiative_id: Option<&str>,
        _board_status: &str,
        _ids: &[String],
        _now_ms: i64,
    ) -> Result<()> {
        anyhow::bail!("atomic TODO reorder is unsupported by this store")
    }
    /// Expected-status CAS for execute starts: a single conditional UPDATE
    /// `SET status = 'running', updated_at = ? WHERE id = ? AND status <>
    /// 'running'`. Returns `true` only when this caller won the claim;
    /// `false` covers both "id not found" and "already running" (someone
    /// else owns the todo right now) — the TOCTOU-closed replacement for a
    /// read-then-patch pair.
    async fn claim_todo_running(&self, id: &str, now_ms: i64) -> Result<bool>;
    /// Atomically excludes concurrent Plan/Execute attempts, allocates the next
    /// version and inserts the run. Execute also sets the todo to `running`.
    /// `false` means the todo was absent or another attempt is running; no
    /// run row is written. Any insert/commit failure rolls the claim back.
    ///
    /// Backends that cannot provide a transaction spanning both tables must
    /// fail before either write instead of degrading to two statements.
    async fn claim_todo_running_with_run(
        &self,
        rec: &ProjectTodoRunRecord,
        now_ms: i64,
    ) -> Result<bool>;
    /// Expected-status CAS variant of `patch_todo`: applies the patch only
    /// when the row's current status equals `when` (and the id exists).
    /// Returns `true` iff applied. Cross-backend caveat: SQLite counts
    /// matched rows (a byte-identical rewrite reports `true`), MySQL counts
    /// changed rows only (the same rewrite reports `false`, indistinguishable
    /// from a lost CAS) — callers must set `patch.status` different from
    /// `when` and treat `false` as "not applied"; both readings are lossless.
    async fn patch_todo_when(
        &self,
        id: &str,
        when: ProjectTodoStatus,
        patch: &ProjectTodoPatch,
        now_ms: i64,
    ) -> Result<bool>;
    /// Transactional cascade: the todo's runs, then the todo.
    async fn delete_todo(&self, id: &str) -> Result<bool>;
    async fn get_todo(&self, id: &str) -> Result<Option<ProjectTodoRecord>>;
    async fn get_todo_summary(&self, _id: &str) -> Result<Option<ProjectTodoSummary>> {
        anyhow::bail!("bounded project todo inspection is unsupported by this store")
    }
    /// `initiative_id == None` lists ALL todos (backlog included); ordered by
    /// `created_at`.
    async fn list_todos(&self, initiative_id: Option<&str>) -> Result<Vec<ProjectTodoRecord>>;
    async fn list_todo_assignments(&self, todo_id: &str) -> Result<Vec<ProjectAssignment>>;
    async fn latest_todo_assignments(&self) -> Result<Vec<ProjectAssignment>>;
    async fn link_todo_execution(&self, assignment: &ProjectAssignment) -> Result<()>;
    async fn unlink_todo_execution(&self, todo_id: &str, execution_id: &str) -> Result<bool>;

    // ---- todo runs ----

    async fn create_todo_run(&self, rec: &ProjectTodoRunRecord) -> Result<()>;
    /// Commit a terminal run and its corresponding todo state in one transaction.
    async fn finish_todo_run(
        &self,
        _id: &str,
        _patch: &ProjectTodoRunPatch,
        _now_ms: i64,
    ) -> Result<bool> {
        anyhow::bail!("atomic project run finalization is unsupported by this store")
    }
    async fn patch_todo_run(
        &self,
        id: &str,
        patch: &ProjectTodoRunPatch,
        now_ms: i64,
    ) -> Result<bool>;
    /// Expected-status CAS variant of `patch_todo_run`: applies the patch
    /// only when the run row's current status equals `when` (and the id
    /// exists). Terminal rows keep their label when a stale convergence
    /// races the driver's own close. Same cross-backend caveat as
    /// `patch_todo_when` (matched- vs changed-rows): keep the patched status
    /// different from `when` and treat `false` as "not applied".
    async fn patch_todo_run_when(
        &self,
        id: &str,
        when: ProjectTodoRunStatus,
        patch: &ProjectTodoRunPatch,
        now_ms: i64,
    ) -> Result<bool>;
    async fn get_todo_run(&self, id: &str) -> Result<Option<ProjectTodoRunRecord>>;
    async fn get_todo_run_summary(&self, _id: &str) -> Result<Option<ProjectTodoRunSummary>> {
        anyhow::bail!("bounded project run inspection is unsupported by this store")
    }
    /// Newest version first.
    async fn list_todo_runs(&self, todo_id: &str) -> Result<Vec<ProjectTodoRunRecord>>;
    async fn list_todo_runs_page(
        &self,
        _todo_id: &str,
        _before_version: Option<i64>,
        _limit: u32,
    ) -> Result<crate::ProjectTodoRunPage> {
        anyhow::bail!("bounded project run pagination is unsupported by this store")
    }
    async fn project_text_chunk(
        &self,
        _record_kind: &str,
        _owner_id: &str,
        _id: &str,
        _field: &str,
        _offset: u64,
        _max_bytes: usize,
    ) -> Result<Option<crate::PayloadChunkRecord>> {
        anyhow::bail!("bounded project text reads are unsupported by this store")
    }
    /// Every run row currently in the `running` state, across todos and
    /// kinds. Powers the opportunistic stale-run sweep (a running row whose
    /// driver no longer exists after a restart/panic).
    async fn list_running_todo_runs(&self) -> Result<Vec<ProjectTodoRunRecord>>;
    /// `COALESCE(MAX(version), 0) + 1` for the todo — 1 for a fresh todo.
    async fn next_todo_version(&self, todo_id: &str) -> Result<i64>;
}
