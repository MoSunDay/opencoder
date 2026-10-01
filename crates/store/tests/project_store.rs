//! Functional tests for the project-module store API (goals / initiatives /
//! todos / runs) against the libsql backend, exercised through the
//! `Arc<dyn ProjectStore>` seam upper layers will use.
//!
//! Behavior contracts:
//! - goal_crud_patch_and_missing_id: CRUD + patch (title/detail/status/sort),
//!   patch/delete of a missing id returns false
//! - initiative_crud_and_goal_filter: CRUD under a goal + list_initiatives filter
//! - todo_patch_semantics_including_clear_to_null: Option<Option<String>>
//!   clears plan_md/initiative_id to NULL; backlog vs initiative todos list
//! - claim_todo_running_cas_and_running_run_listing: expected-status CAS
//!   (planned -> claim true, re-claim/unknown -> false, no re-stamp) and
//!   list_running_todo_runs filters to running rows only
//! - conditional_patch_cas_applies_only_in_expected_state: patch_todo_when /
//!   patch_todo_run_when apply only while the row still holds the expected
//!   status (claim-rollback, plan-writeback race, terminal-run convergence)
//! - run_versions_and_listing_order: next_todo_version numbering (empty = 1),
//!   newest-first listing, patch stamps status/finished_at
//! - executor_dimension_round_trips: todo executor_kind/ref/spec and run
//!   executor_kind/capability_id/plan_id/output_ref persist exactly (team todo
//!   + dag run), unknown kind text fails closed on read
//! - deletion: delete_goal detaches initiatives and retains TODOs/runs;
//!   nonempty initiatives reject deletion; delete_todo removes its runs
//! - reopen_is_idempotent_and_serves_v15: second `open` on the same file
//!   migrates 14→15 cleanly and the project tables keep working
//! - libsql_store_coerces_to_project_store: compile-level trait-object check

use std::sync::Arc;

use opencoder_store::{
    LibsqlStore, ProjectExecutorKind, ProjectGoalPatch, ProjectGoalRecord, ProjectGoalStatus,
    ProjectInitiativePatch, ProjectInitiativeRecord, ProjectInitiativeStatus, ProjectStore,
    ProjectTodoPatch, ProjectTodoRecord, ProjectTodoRunKind, ProjectTodoRunPatch,
    ProjectTodoRunRecord, ProjectTodoRunStatus, ProjectTodoStatus,
};

async fn fresh() -> (tempfile::TempDir, Arc<LibsqlStore>, Arc<dyn ProjectStore>) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(LibsqlStore::open(dir.path().join("test.db")).await.unwrap());
    let iface: Arc<dyn ProjectStore> = store.clone();
    (dir, store, iface)
}

fn goal(id: &str, sort: i64, created_at: i64) -> ProjectGoalRecord {
    ProjectGoalRecord {
        id: id.to_string(),
        title: format!("goal {id}"),
        detail_md: None,
        status: ProjectGoalStatus::Active,
        sort,
        created_at,
        updated_at: created_at,
    }
}

fn initiative(id: &str, goal_id: &str, sort: i64, created_at: i64) -> ProjectInitiativeRecord {
    ProjectInitiativeRecord {
        id: id.to_string(),
        goal_id: Some(goal_id.to_string()),
        title: format!("initiative {id}"),
        detail_md: None,
        status: ProjectInitiativeStatus::Planned,
        sort,
        created_at,
        updated_at: created_at,
    }
}

fn todo(id: &str, initiative_id: Option<&str>, created_at: i64) -> ProjectTodoRecord {
    ProjectTodoRecord {
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

async fn run(store: &dyn ProjectStore, id: &str, todo_id: &str, created_at: i64) {
    let version = store.next_todo_version(todo_id).await.unwrap();
    store
        .create_todo_run(&ProjectTodoRunRecord {
            input_snapshot: None,
            trace_manifest: None,
            id: id.to_string(),
            todo_id: todo_id.to_string(),
            kind: ProjectTodoRunKind::Plan,
            version,
            plan_md: Some("plan".to_string()),
            output_md: None,
            agent: "plan".to_string(),
            executor_kind: ProjectExecutorKind::Agent,
            capability_id: None,
            plan_id: None,
            output_ref: None,
            session_id: Some(format!("sess-{id}")),
            status: ProjectTodoRunStatus::Running,
            started_at: created_at,
            finished_at: None,
            created_at,
        })
        .await
        .unwrap();
}

// Compile-level: the concrete store coerces to the trait object upper
// layers hold (`Arc<dyn ProjectStore>`).
#[allow(dead_code)]
fn libsql_store_coerces_to_project_store(store: Arc<LibsqlStore>) -> Arc<dyn ProjectStore> {
    store
}

// The v20 executor dimension must persist exactly through the libsql backend:
// a team todo keeps its executor_ref + inline executor_spec, and a dag run
// claimed under it keeps brain provenance (capability_id/plan_id) and the
// workflow artifact root (output_ref). Unknown kind text in the row is
// corruption and fails closed on read.

#[path = "project_store/suite_1.rs"]
mod suite_1;
#[path = "project_store/suite_2.rs"]
mod suite_2;
#[path = "project_store/suite_3.rs"]
mod suite_3;
