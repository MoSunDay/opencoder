//! Upgrade and backup/reopen evidence uses only databases created by this test.
use opencoder_store::{LibsqlStore, ProjectStore, ProjectTodoRunPatch};

#[tokio::test]
async fn v20_backup_preserves_history_and_v21_replay_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v20.db");
    let backup = dir.path().join("v20-backup.db");
    {
        let db = libsql::Builder::new_local(&path).build().await.unwrap();
        let conn = db.connect().unwrap();
        conn.execute_batch(
            "CREATE TABLE schema_version(version INTEGER NOT NULL);
             INSERT INTO schema_version VALUES(20);
             CREATE TABLE project_todo_runs (
               id TEXT PRIMARY KEY, todo_id TEXT NOT NULL, kind TEXT NOT NULL,
               version INTEGER NOT NULL, plan_md TEXT, output_md TEXT,
               agent TEXT NOT NULL, session_id TEXT, status TEXT NOT NULL,
               started_at INTEGER NOT NULL, finished_at INTEGER,
               created_at INTEGER NOT NULL, executor_kind TEXT NOT NULL DEFAULT 'agent',
               capability_id TEXT, plan_id TEXT, output_ref TEXT);
             INSERT INTO project_todo_runs VALUES
               ('prun-legacy','todo-legacy','execute',7,'old plan','old output',
                'act','session-legacy','done',11,12,11,'agent',NULL,NULL,NULL);",
        )
        .await
        .unwrap();
    }
    std::fs::copy(&path, &backup).unwrap();
    let original_backup = std::fs::read(&backup).unwrap();
    let store = LibsqlStore::open(&path).await.unwrap();
    let legacy = store.get_todo_run("prun-legacy").await.unwrap().unwrap();
    assert_eq!(legacy.version, 7);
    assert_eq!(legacy.plan_md.as_deref(), Some("old plan"));
    assert_eq!(legacy.output_md.as_deref(), Some("old output"));
    assert_eq!(legacy.session_id.as_deref(), Some("session-legacy"));
    assert!(legacy.input_snapshot.is_none());
    assert!(legacy.trace_manifest.is_none());
    let mut current = legacy.clone();
    current.id = "prun-new".into();
    current.version = 8;
    current.input_snapshot = Some("{\"request\":\"retained input 界\"}".into());
    current.trace_manifest = Some("{\"schema\":1,\"complete\":false}".into());
    store.create_todo_run(&current).await.unwrap();
    let complete = "{\"schema\":1,\"complete\":true,\"messages_through\":9}";
    store
        .patch_todo_run(
            &current.id,
            &ProjectTodoRunPatch {
                trace_manifest: Some(complete.into()),
                ..Default::default()
            },
            20,
        )
        .await
        .unwrap();
    drop(store);
    let reopened = LibsqlStore::open(&path).await.unwrap();
    let retained = reopened.get_todo_run(&current.id).await.unwrap().unwrap();
    assert_eq!(retained.input_snapshot, current.input_snapshot);
    assert_eq!(retained.trace_manifest.as_deref(), Some(complete));
    assert_eq!(retained.output_md, current.output_md);
    let conn = reopened.conn().await.unwrap();
    let mut rows = conn
        .query("SELECT version FROM schema_version", ())
        .await
        .unwrap();
    assert_eq!(
        rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(),
        24
    );
    assert_eq!(std::fs::read(&backup).unwrap(), original_backup);
    // Restoring the unchanged backup into a separate path is independently viable.
    let restored = dir.path().join("restored.db");
    std::fs::copy(&backup, &restored).unwrap();
    let restored = LibsqlStore::open(restored).await.unwrap();
    assert!(restored.get_todo_run("prun-new").await.unwrap().is_none());
    let restored_legacy = restored.get_todo_run("prun-legacy").await.unwrap().unwrap();
    assert_eq!(restored_legacy.output_md, legacy.output_md);
    assert!(restored_legacy.input_snapshot.is_none());
}
