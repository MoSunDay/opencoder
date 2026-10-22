use opencoder_store::{SessionMeta, Store, TursoStore};

#[tokio::test]
async fn inventory_tracks_lane_changes_deletion_and_transaction_rollback() {
    let store = TursoStore::open_memory().await.unwrap();
    let conn = store.conn().await.unwrap();
    store
        .create_session(&SessionMeta {
            id: "agent-test".into(),
            created_at: 1,
            updated_at: 1,
            ..Default::default()
        })
        .await
        .unwrap();
    let full = store.inventory_sources(None).await.unwrap();
    assert!(full.full);
    assert_eq!(full.sessions.len(), 1);
    let revision = full.revision;
    conn.execute(
        "UPDATE sessions SET updated_at=2,summary='ignored',kind=kind WHERE id='agent-test'",
        (),
    )
    .await
    .unwrap();
    let idle = store.inventory_sources(Some(revision)).await.unwrap();
    assert_eq!(idle.revision, revision);
    assert!(idle.sessions.is_empty());
    conn.execute("BEGIN IMMEDIATE", ()).await.unwrap();
    conn.execute("DELETE FROM sessions WHERE id='agent-test'", ())
        .await
        .unwrap();
    conn.execute("ROLLBACK", ()).await.unwrap();
    assert_eq!(
        store
            .inventory_sources(Some(revision))
            .await
            .unwrap()
            .revision,
        revision
    );
    conn.execute(
        "UPDATE sessions SET kind='operator' WHERE id='agent-test'",
        (),
    )
    .await
    .unwrap();
    let hidden = store.inventory_sources(Some(revision)).await.unwrap();
    assert!(hidden.sessions.is_empty());
    assert_eq!(hidden.removed_sessions, vec!["agent-test"]);
    conn.execute("UPDATE sessions SET kind='agent' WHERE id='agent-test'", ())
        .await
        .unwrap();
    let restored = store
        .inventory_sources(Some(hidden.revision))
        .await
        .unwrap();
    assert_eq!(restored.sessions.len(), 1);
    store.delete_session("agent-test").await.unwrap();
    let removed = store
        .inventory_sources(Some(restored.revision))
        .await
        .unwrap();
    assert!(removed.sessions.is_empty());
    assert_eq!(removed.removed_sessions, vec!["agent-test"]);
    assert!(
        store
            .inventory_sources(Some(removed.revision + 1))
            .await
            .unwrap()
            .full
    );
}

#[tokio::test]
async fn inventory_never_needs_message_previews_or_project_bodies() {
    let store = TursoStore::open_memory().await.unwrap();
    let conn = store.conn().await.unwrap();
    store
        .create_session(&SessionMeta {
            id: "agent-preview".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    // An invalid message body is harmless to the inventory-only reader.
    conn.execute("INSERT INTO messages(id,session_id,role,blocks_json,usage_json,created_at) VALUES('message','agent-preview','user',printf('%.*c',1048576,'x'),'{}',0)",()).await.unwrap();
    conn.execute("INSERT INTO project_todo_runs(id,todo_id,kind,version,agent,status,started_at,created_at,plan_md,output_md,input_snapshot,trace_manifest) VALUES('run','todo','execute',1,'test','running',0,2,printf('%.*c',1048576,'x'),printf('%.*c',1048576,'x'),'invalid json','invalid json')",()).await.unwrap();
    let full = store.inventory_sources(None).await.unwrap();
    assert_eq!(full.sessions.len(), 1);
    assert_eq!(full.project_runs.len(), 1);
    assert_eq!(full.project_runs[0].created_at, 2);
    conn.execute(
        "UPDATE project_todo_runs SET output_md='changed',status=status WHERE id='run'",
        (),
    )
    .await
    .unwrap();
    assert_eq!(
        store
            .inventory_sources(Some(full.revision))
            .await
            .unwrap()
            .revision,
        full.revision
    );
    conn.execute(
        "UPDATE project_todo_runs SET status='done' WHERE id='run'",
        (),
    )
    .await
    .unwrap();
    let changed = store.inventory_sources(Some(full.revision)).await.unwrap();
    assert_eq!(
        changed.project_runs[0].status,
        opencoder_store::ProjectTodoRunStatus::Done
    );
    conn.execute("DELETE FROM project_todo_runs WHERE id='run'", ())
        .await
        .unwrap();
    let removed = store
        .inventory_sources(Some(changed.revision))
        .await
        .unwrap();
    assert_eq!(removed.removed_runs, vec!["run"]);
}

#[tokio::test]
async fn subagent_membership_changes_invalidate_the_session_lane() {
    let store = TursoStore::open_memory().await.unwrap();
    let conn = store.conn().await.unwrap();
    for id in ["agent-parent", "agent-child"] {
        store
            .create_session(&SessionMeta {
                id: id.into(),
                ..Default::default()
            })
            .await
            .unwrap();
    }
    let before = store.inventory_sources(None).await.unwrap();
    conn.execute("INSERT INTO subagent_tasks(task_id,parent_session_id,child_session_id,agent,prompt,status,started_at) VALUES('task','agent-parent','agent-child','test','','running',0)",()).await.unwrap();
    let child = store
        .inventory_sources(Some(before.revision))
        .await
        .unwrap();
    assert!(child.sessions.is_empty());
    assert_eq!(child.removed_sessions, vec!["agent-child"]);
    conn.execute("DELETE FROM subagent_tasks WHERE task_id='task'", ())
        .await
        .unwrap();
    let restored = store.inventory_sources(Some(child.revision)).await.unwrap();
    assert_eq!(restored.sessions[0].id, "agent-child");
}

#[tokio::test]
async fn inventory_recovers_expired_cursors_and_bootstraps_existing_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("inventory.db");
    let store = TursoStore::open(&path).await.unwrap();
    store
        .create_session(&SessionMeta {
            id: "agent-existing".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let first = store.inventory_sources(None).await.unwrap();
    let conn = store.conn().await.unwrap();
    conn.execute("INSERT INTO inventory_source_changes(seq,source,id) VALUES(100005,'session','agent-existing')", ()).await.unwrap();
    conn.execute("DELETE FROM inventory_source_changes WHERE seq<100005", ())
        .await
        .unwrap();
    let expired = store.inventory_sources(Some(first.revision)).await.unwrap();
    assert!(expired.full);
    assert_eq!(expired.sessions.len(), 1);
    // Simulate an old database with source rows but no derived cursor table.
    for table in ["sessions", "subagent_tasks", "project_todo_runs"] {
        for event in ["insert", "update", "delete"] {
            conn.execute(&format!("DROP TRIGGER inventory_{table}_{event}"), ())
                .await
                .unwrap();
        }
    }
    conn.execute("DROP TABLE inventory_source_changes", ())
        .await
        .unwrap();
    drop(conn);
    drop(store);
    let reopened = TursoStore::open(&path).await.unwrap();
    assert_eq!(
        reopened.inventory_sources(None).await.unwrap().sessions[0].id,
        "agent-existing"
    );
    let initial = reopened.inventory_sources(None).await.unwrap();
    reopened.delete_session("agent-existing").await.unwrap();
    assert_eq!(
        reopened
            .inventory_sources(Some(initial.revision))
            .await
            .unwrap()
            .removed_sessions,
        vec!["agent-existing"]
    );
}
