//! `SessionListItem` carries derived subagent-task status counts so the `/task`
//! picker can show per-session status without a separate query.
//!
//! The counts are aggregated from `subagent_tasks` by `parent_session_id` at
//! list time: `subagent_running` counts `Running` rows (in-flight children),
//! `subagent_cancelled` counts `Cancelled` rows (interrupted children pending
//! replay on the next user turn). `Completed`/`Failed` rows are terminal and
//! contribute to neither count. The existing subagent-exclusion filter
//! (`include_subagents=false`) must stay intact alongside the aggregation.

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
