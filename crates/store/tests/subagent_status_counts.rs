//! `SessionListItem` carries derived subagent-task status counts so the `/task`
//! picker can show per-session status without a separate query.
//!
//! The counts are aggregated from `subagent_tasks` by `parent_session_id` at
//! list time: `subagent_running` counts `Running` rows (in-flight children),
//! `subagent_cancelled` counts `Cancelled` rows (interrupted children pending
//! replay on the next user turn). `Completed`/`Failed` rows are terminal and
//! contribute to neither count. The existing subagent-exclusion filter
//! (`include_subagents=false`) must stay intact alongside the aggregation.
//!
//! Also hosts the subagent-task contract tests moved from the (now split)
//! `store_integration.rs`: task CRUD round-trip, per-parent filtering, and
//! `SubagentStatus` parse/as_str round-trips.

use tempfile::TempDir;

use opencoder_store::{
    LibsqlStore, SessionFilter, SessionMeta, Store, SubagentStatus, SubagentTaskRecord,
    TASK_TYPE_SUBAGENT,
};

async fn mem() -> LibsqlStore {
    LibsqlStore::open_memory().await.unwrap()
}

fn meta(id: &str, task_type: Option<&str>) -> SessionMeta {
    SessionMeta {
        id: id.into(),
        title: Some(id.into()),
        agent: Some("act".into()),
        model: Some("m".into()),
        workdir_hash: None,
        created_at: 0,
        updated_at: 0,
        summary: None,
        summary_seq: None,
        handoff_seq: None,
        handoff_plan: None,
        skill: None,
        task_type: task_type.map(str::to_string),
    }
}

fn task(parent: &str, task_id: &str, child: &str) -> SubagentTaskRecord {
    SubagentTaskRecord {
        task_id: task_id.into(),
        parent_session_id: parent.into(),
        child_session_id: child.into(),
        parent_message_id: None,
        agent: "act".into(),
        prompt: "delegate".into(),
        result: None,
        status: SubagentStatus::Running,
        ok: None,
        started_at: 0,
        completed_at: None,
    }
}

#[tokio::test]
async fn list_aggregates_running_cancelled_completed_mix() {
    let store = mem().await;
    store.create_session(&meta("p1", None)).await.unwrap();
    store.create_session(&meta("p2", None)).await.unwrap();
    store
        .create_session(&meta("c1", Some(TASK_TYPE_SUBAGENT)))
        .await
        .unwrap();

    // p1: 2 running + 1 cancelled + 2 completed (terminal).
    for i in 0..2 {
        let tid = format!("run-{i}");
        store
            .create_subagent_task(&task("p1", &tid, "c1"))
            .await
            .unwrap();
    }
    store
        .create_subagent_task(&task("p1", "cancel-1", "c1"))
        .await
        .unwrap();
    store.cancel_subagent_task("cancel-1").await.unwrap();
    for i in 0..2 {
        let tid = format!("done-{i}");
        store
            .create_subagent_task(&task("p1", &tid, "c1"))
            .await
            .unwrap();
        store
            .complete_subagent_task(&tid, "result-ok", true)
            .await
            .unwrap();
    }

    let listed = store
        .list_sessions(&SessionFilter {
            limit: 100,
            ..Default::default()
        })
        .await
        .unwrap();

    // Default filter excludes the subagent child.
    let ids: Vec<&str> = listed.iter().map(|i| i.id.as_str()).collect();
    assert!(
        !ids.contains(&"c1"),
        "subagent child must stay excluded, got {:?}",
        ids
    );

    let p1 = listed.iter().find(|i| i.id == "p1").expect("p1 listed");
    assert_eq!(
        p1.subagent_running, 2,
        "running count must match the 2 in-flight tasks"
    );
    assert_eq!(
        p1.subagent_cancelled, 1,
        "cancelled count must match the 1 interrupted task"
    );

    let p2 = listed.iter().find(|i| i.id == "p2").expect("p2 listed");
    assert_eq!(p2.subagent_running, 0, "no tasks -> 0 running");
    assert_eq!(p2.subagent_cancelled, 0, "no tasks -> 0 cancelled");
}

#[tokio::test]
async fn list_with_subagents_reports_zero_counts_for_children() {
    let store = mem().await;
    store.create_session(&meta("p1", None)).await.unwrap();
    store
        .create_session(&meta("c1", Some(TASK_TYPE_SUBAGENT)))
        .await
        .unwrap();
    store
        .create_subagent_task(&task("p1", "run-1", "c1"))
        .await
        .unwrap();

    let listed = store
        .list_sessions(&SessionFilter {
            limit: 100,
            include_subagents: true,
            ..Default::default()
        })
        .await
        .unwrap();
    let p1 = listed.iter().find(|i| i.id == "p1").unwrap();
    let c1 = listed.iter().find(|i| i.id == "c1").unwrap();
    assert_eq!(p1.subagent_running, 1, "parent aggregates its in-flight task");
    assert_eq!(
        c1.subagent_running, 0,
        "child session (a subagent itself) has no tasks as parent"
    );
    assert_eq!(c1.subagent_cancelled, 0);
}

async fn fresh() -> (TempDir, LibsqlStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = LibsqlStore::open(dir.path().join("test.db")).await.unwrap();
    (dir, store)
}

async fn make_session(store: &LibsqlStore, id: &str, now: i64) {
    let meta = SessionMeta {
        id: id.to_string(),
        title: Some(format!("title-{id}")),
        agent: Some("act".into()),
        model: Some("glm-5.2".into()),
        workdir_hash: Some("h".into()),
        created_at: now,
        updated_at: now,
        summary: None,
        summary_seq: None,
        handoff_seq: None,
        handoff_plan: None,
        skill: None,
        task_type: None,
    };
    store.create_session(&meta).await.unwrap();
}

#[tokio::test]
async fn subagent_task_crud_roundtrip() {

    let (_dir, store) = fresh().await;
    // Seed session rows so the FK constraints on parent/child resolve.
    make_session(&store, "parent-sess", 0).await;
    make_session(&store, "sub-sess-001", 0).await;

    let rec = SubagentTaskRecord {
        task_id: "task-001".into(),
        parent_session_id: "parent-sess".into(),
        child_session_id: "sub-sess-001".into(),
        parent_message_id: Some("msg-42".into()),
        agent: "explore".into(),
        prompt: "find all TODO comments".into(),
        result: None,
        status: SubagentStatus::Running,
        ok: None,
        started_at: 1000,
        completed_at: None,
    };
    store.create_subagent_task(&rec).await.unwrap();

    // List as Running.
    let rows = store.list_subagent_tasks("parent-sess").await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].task_id, "task-001");
    assert_eq!(rows[0].child_session_id, "sub-sess-001");
    assert_eq!(rows[0].agent, "explore");
    assert!(matches!(rows[0].status, SubagentStatus::Running));
    assert!(rows[0].result.is_none());
    assert!(rows[0].ok.is_none());

    // Complete it.
    store
        .complete_subagent_task("task-001", "found 5 TODOs", true)
        .await
        .unwrap();

    // List again — must reflect completion.
    let rows = store.list_subagent_tasks("parent-sess").await.unwrap();
    assert_eq!(rows.len(), 1);
    assert!(matches!(rows[0].status, SubagentStatus::Completed));
    assert_eq!(rows[0].result.as_deref(), Some("found 5 TODOs"));
    assert_eq!(rows[0].ok, Some(true));
    assert!(rows[0].completed_at.is_some(), "completed_at must be set");
}

#[tokio::test]
async fn subagent_task_list_filters_by_parent() {

    let (_dir, store) = fresh().await;

    for (tid, parent) in [("t-a", "sess-a"), ("t-b", "sess-b"), ("t-c", "sess-a")] {
        make_session(&store, parent, 0).await;
        make_session(&store, &format!("child-{tid}"), 0).await;
        let rec = SubagentTaskRecord {
            task_id: tid.into(),
            parent_session_id: parent.into(),
            child_session_id: format!("child-{tid}"),
            parent_message_id: None,
            agent: "build".into(),
            prompt: format!("prompt-{tid}"),
            result: None,
            status: SubagentStatus::Running,
            ok: None,
            started_at: 2000,
            completed_at: None,
        };
        store.create_subagent_task(&rec).await.unwrap();
    }

    let a_rows = store.list_subagent_tasks("sess-a").await.unwrap();
    assert_eq!(a_rows.len(), 2, "sess-a should have 2 tasks");
    let b_rows = store.list_subagent_tasks("sess-b").await.unwrap();
    assert_eq!(b_rows.len(), 1, "sess-b should have 1 task");
    let none_rows = store.list_subagent_tasks("sess-c").await.unwrap();
    assert!(none_rows.is_empty(), "sess-c should have 0 tasks");
}

#[tokio::test]
async fn subagent_status_parse_and_as_str() {
    assert_eq!(SubagentStatus::parse("running"), SubagentStatus::Running);
    assert_eq!(
        SubagentStatus::parse("completed"),
        SubagentStatus::Completed
    );
    assert_eq!(SubagentStatus::parse("failed"), SubagentStatus::Failed);
    assert_eq!(
        SubagentStatus::parse("cancelled"),
        SubagentStatus::Cancelled
    );
    assert_eq!(SubagentStatus::parse("bogus"), SubagentStatus::Running);
    // Regression: "unknown" must round-trip, not fall through to Running.
    assert_eq!(SubagentStatus::parse("unknown"), SubagentStatus::Unknown);
    assert_eq!(SubagentStatus::Unknown.as_str(), "unknown");
    assert_eq!(
        SubagentStatus::parse(SubagentStatus::Unknown.as_str()),
        SubagentStatus::Unknown
    );
    assert_eq!(SubagentStatus::Running.as_str(), "running");
    assert_eq!(SubagentStatus::Completed.as_str(), "completed");
    assert_eq!(SubagentStatus::Failed.as_str(), "failed");
    assert_eq!(SubagentStatus::Cancelled.as_str(), "cancelled");
}
