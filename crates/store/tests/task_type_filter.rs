//! `/task` listing filters subagent child sessions via the `task_type` column.
//!
//! The `task_type` column is the canonical marker distinguishing parent
//! (top-level) sessions from subagent children. This covers the race where a
//! child session row exists but no `subagent_tasks` row was written yet
//! (process crashed between `create_session` and `create_subagent_task`):
//! such an orphaned child is still excluded because its `task_type = 'subagent'`.

use opencoder_store::{
    LibsqlStore, SessionFilter, SessionMeta, Store, TASK_TYPE_PARENT, TASK_TYPE_SUBAGENT,
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
        autopilot_mode: None,
        workdir_hash: None,
        created_at: 0,
        updated_at: 0,
        summary: None,
        summary_seq: None,
        summary_images: vec![],
        handoff_seq: None,
        handoff_plan: None,
        skill: None,
        task_type: task_type.map(str::to_string),
        requirement: None,
        plan_snapshot: None,
        plan_input_count: 0,
    }
}

#[tokio::test]
async fn list_excludes_subagent_children_by_task_type() {
    let store = mem().await;
    // Two normal parent sessions.
    store.create_session(&meta("p1", None)).await.unwrap();
    store.create_session(&meta("p2", None)).await.unwrap();
    // A child session explicitly typed 'subagent' but with NO subagent_tasks
    // row — the crash-race orphan that the old NOT-EXISTS-only filter would leak.
    store
        .create_session(&meta("c1", Some(TASK_TYPE_SUBAGENT)))
        .await
        .unwrap();

    // Default filter (include_subagents=false): only parents appear.
    let listed = store
        .list_sessions(&SessionFilter {
            limit: 100,
            ..Default::default()
        })
        .await
        .unwrap();
    let ids: Vec<&str> = listed.iter().map(|i| i.id.as_str()).collect();
    assert!(ids.contains(&"p1"), "parent p1 must be listed");
    assert!(ids.contains(&"p2"), "parent p2 must be listed");
    assert!(
        !ids.contains(&"c1"),
        "subagent child c1 must NOT be listed, got {:?}",
        ids
    );
}

#[tokio::test]
async fn list_includes_subagents_when_requested() {
    let store = mem().await;
    store.create_session(&meta("p1", None)).await.unwrap();
    store
        .create_session(&meta("c1", Some(TASK_TYPE_SUBAGENT)))
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
    let ids: Vec<&str> = listed.iter().map(|i| i.id.as_str()).collect();
    assert!(ids.contains(&"p1") && ids.contains(&"c1"));
}

#[tokio::test]
async fn parent_session_persists_default_task_type() {
    // A parent (task_type=None) is stored and read back with the DB default
    // 'parent'. get_session returns the raw column value.
    let store = mem().await;
    store.create_session(&meta("p1", None)).await.unwrap();
    let got = store.get_session("p1").await.unwrap().unwrap();
    assert_eq!(
        got.task_type.as_deref(),
        Some(TASK_TYPE_PARENT),
        "parent session must persist task_type='parent'"
    );

    store
        .create_session(&meta("c1", Some(TASK_TYPE_SUBAGENT)))
        .await
        .unwrap();
    let got = store.get_session("c1").await.unwrap().unwrap();
    assert_eq!(
        got.task_type.as_deref(),
        Some(TASK_TYPE_SUBAGENT),
        "subagent child must persist task_type='subagent'"
    );
}
