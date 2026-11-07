//! Project-module contracts for the rdb (RocksDB) backend, ported from the
//! SQL suites: catalog CRUD, atomic claims/runs, tag reconciliation,
//! comments, summary history, people validation and board reordering — all
//! through the `Arc<dyn ProjectStore>` seam upper layers use.

#[path = "rdb_project/suite_atomic.rs"]
mod suite_atomic;
#[path = "rdb_project/suite_comment_mutations.rs"]
mod suite_comment_mutations;
#[path = "rdb_project/suite_comments.rs"]
mod suite_comments;
#[path = "rdb_project/suite_crud.rs"]
mod suite_crud;
#[path = "rdb_project/suite_order.rs"]
mod suite_order;
#[path = "rdb_project/suite_people.rs"]
mod suite_people;
#[path = "rdb_project/suite_runs.rs"]
mod suite_runs;
#[path = "rdb_project/suite_runs_finish.rs"]
mod suite_runs_finish;
#[path = "rdb_project/suite_summaries.rs"]
mod suite_summaries;
#[path = "rdb_project/suite_tags.rs"]
mod suite_tags;
#[path = "rdb_project/suite_tags_global.rs"]
mod suite_tags_global;
#[path = "rdb_project/suite_tags_scope.rs"]
mod suite_tags_scope;

use std::sync::Arc;

use opencoder_store::rdb_store::RdbStore;
use opencoder_store::{
    ProjectExecutorKind, ProjectGoalRecord, ProjectGoalStatus, ProjectInitiativeRecord,
    ProjectStore, ProjectTodoPatch, ProjectTodoRecord, ProjectTodoRunKind, ProjectTodoRunPatch,
    ProjectTodoRunRecord, ProjectTodoRunStatus, ProjectTodoStatus,
};

/// Status-only todo patch (`people` stays an empty no-op assignment set).
pub(crate) fn tpatch(status: Option<ProjectTodoStatus>) -> ProjectTodoPatch {
    ProjectTodoPatch {
        people: Default::default(),
        status,
        ..Default::default()
    }
}

/// Status/finish-only run patch.
pub(crate) fn rpatch(
    status: Option<ProjectTodoRunStatus>,
    finished_at: Option<i64>,
) -> ProjectTodoRunPatch {
    ProjectTodoRunPatch {
        status,
        finished_at,
        ..Default::default()
    }
}

pub(crate) async fn mem() -> Arc<RdbStore> {
    Arc::new(RdbStore::open_memory().await.unwrap())
}

pub(crate) async fn mem_iface() -> Arc<dyn ProjectStore> {
    let store = mem().await;
    store.clone()
}

pub(crate) fn goal(id: &str, sort: i64, created_at: i64) -> ProjectGoalRecord {
    ProjectGoalRecord {
        summary: Default::default(),
        people: Default::default(),
        id: id.to_string(),
        title: format!("goal {id}"),
        detail_md: None,
        status: ProjectGoalStatus::Active,
        sort,
        created_at,
        updated_at: created_at,
    }
}

pub(crate) fn initiative(
    id: &str,
    goal_id: Option<&str>,
    sort: i64,
    created_at: i64,
) -> ProjectInitiativeRecord {
    ProjectInitiativeRecord {
        summary: Default::default(),
        people: Default::default(),
        id: id.to_string(),
        goal_id: goal_id.map(str::to_string),
        title: format!("initiative {id}"),
        detail_md: None,
        sort,
        created_at,
        updated_at: created_at,
    }
}

pub(crate) fn todo(id: &str, initiative_id: Option<&str>, created_at: i64) -> ProjectTodoRecord {
    ProjectTodoRecord {
        people: Default::default(),
        id: id.to_string(),
        initiative_id: initiative_id.map(str::to_string),
        title: format!("todo {id}"),
        draft: format!("draft {id}"),
        plan_md: None,
        status: ProjectTodoStatus::Draft,
        board_status: "backlog".into(),
        position: created_at,
        capability_id: None,
        agent: "act".to_string(),
        executor_kind: ProjectExecutorKind::Agent,
        executor_ref: None,
        executor_spec: None,
        active_session_id: None,
        created_at,
        updated_at: created_at,
    }
}

pub(crate) fn run_record(
    id: &str,
    todo_id: &str,
    version: i64,
    created_at: i64,
    kind: ProjectTodoRunKind,
) -> ProjectTodoRunRecord {
    ProjectTodoRunRecord {
        input_snapshot: None,
        trace_manifest: None,
        id: id.to_string(),
        todo_id: todo_id.to_string(),
        kind,
        version,
        plan_md: None,
        output_md: None,
        agent: "act".to_string(),
        executor_kind: ProjectExecutorKind::Agent,
        capability_id: None,
        plan_id: None,
        output_ref: None,
        session_id: Some(format!("sess-{id}")),
        status: ProjectTodoRunStatus::Running,
        started_at: created_at,
        finished_at: None,
        created_at,
    }
}

pub(crate) async fn run(p: &dyn ProjectStore, id: &str, todo_id: &str, created_at: i64) {
    let version = p.next_todo_version(todo_id).await.unwrap();
    p.create_todo_run(&run_record(
        id,
        todo_id,
        version,
        created_at,
        ProjectTodoRunKind::Plan,
    ))
    .await
    .unwrap();
}

// Compile-level: the concrete store coerces to the trait object upper
// layers hold (`Arc<dyn ProjectStore>`).
#[allow(dead_code)]
fn rdb_store_coerces_to_project_store(store: Arc<RdbStore>) -> Arc<dyn ProjectStore> {
    store
}
