//! Execution-ledger contracts for the rdb (RocksDB) backend, ported from
//! the SQL suites (`tests/dag_store`, `tests/nodes_store*`,
//! `tests/node_tasks_query`, `tests/nodes_lost_converge`,
//! `tests/node_dialogs_clear`, `tests/schedule_*`, `tests/team_runs` and
//! the `delete_sessions` seam): node-task queue semantics, DAG
//! definitions/runs/events, schedules, team topic runs and the
//! session-cascading deletes.

#[path = "rdb_execution/suite_dag.rs"]
mod suite_dag;
#[path = "rdb_execution/suite_dag_runs.rs"]
mod suite_dag_runs;
#[path = "rdb_execution/suite_dialogs.rs"]
mod suite_dialogs;
#[path = "rdb_execution/suite_lost.rs"]
mod suite_lost;
#[path = "rdb_execution/suite_node_tasks.rs"]
mod suite_node_tasks;
#[path = "rdb_execution/suite_node_tasks_query.rs"]
mod suite_node_tasks_query;
#[path = "rdb_execution/suite_schedules_team.rs"]
mod suite_schedules_team;

use opencoder_dag::DagRunStatus;
use opencoder_store::rdb_store::RdbStore;
use opencoder_store::Store;
use opencoder_store::{DagDefRecord, DagRunRecord, NodeRecord, NodeTaskRecord, NodeTaskStatus};

pub(crate) async fn mem() -> RdbStore {
    RdbStore::open_memory().await.unwrap()
}

pub(crate) async fn register(store: &RdbStore, name: &str, now_ms: i64) -> NodeRecord {
    store
        .register_node(name, Some("v1"), Some("/tmp/wd"), None, now_ms)
        .await
        .unwrap()
}

/// The nodes_store-style dispatch: synthetic `sess-<seq>` session, titled
/// task with agent+model (exercises the full column surface).
pub(crate) async fn dispatch(
    store: &RdbStore,
    task_id: &str,
    session_seq: usize,
    node_id: &str,
    created_at: i64,
) {
    store
        .dispatch_node_task(
            task_id,
            &format!("sess-{session_seq}"),
            node_id,
            Some(format!("task {task_id}").as_str()),
            format!("prompt-{task_id}").as_str(),
            Some("build"),
            Some("glm-5.2"),
            created_at,
        )
        .await
        .unwrap();
}

/// The query-suite-style dispatch: minimal args, synthetic `sess-<id>`
/// session, returns the stored record.
pub(crate) async fn dispatch_simple(
    store: &RdbStore,
    id: &str,
    node: &str,
    created_at: i64,
) -> NodeTaskRecord {
    store
        .dispatch_node_task(
            id,
            &format!("sess-{id}"),
            node,
            Some(format!("title {id}").as_str()),
            &format!("prompt-{id}"),
            None,
            None,
            created_at,
        )
        .await
        .unwrap()
}

/// The cancelling-row helper shared by the transition suites.
pub(crate) async fn claim_nothing(store: &RdbStore, node_id: &str) -> bool {
    matches!(store.claim_next_node_task(node_id, 2050).await, Ok(None))
}

// ── DAG helpers (shared by suite_dag / suite_dag_runs) ────────────────────

pub(crate) fn dag_def(id: &str, name: &str, spec_json: &str, at: i64) -> DagDefRecord {
    DagDefRecord {
        id: id.into(),
        name: name.into(),
        spec_json: spec_json.into(),
        created_at: at,
        updated_at: at,
    }
}

pub(crate) fn dag_run(
    id: &str,
    node: Option<&str>,
    created_at: i64,
    spec_json: &str,
) -> DagRunRecord {
    DagRunRecord {
        id: id.into(),
        dag_id: "dag-1".into(),
        name: format!("run-{id}"),
        spec_json: spec_json.into(),
        node_id: node.map(str::to_string),
        status: DagRunStatus::Pending,
        error: None,
        created_at,
        claimed_at: None,
        finished_at: None,
    }
}

pub(crate) async fn dispatch_dag(store: &RdbStore, id: &str, node: Option<&str>, created_at: i64) {
    store
        .dispatch_dag_run(&dag_run(id, node, created_at, r#"{"v":1}"#))
        .await
        .unwrap();
}
