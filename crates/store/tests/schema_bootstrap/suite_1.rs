use super::*;

#[tokio::test]
async fn synchronous_is_normal_after_open() {
    let dir = tempfile::tempdir().unwrap();
    let store = LibsqlStore::open(dir.path().join("cold.db")).await.unwrap();
    let conn = store.conn().await.unwrap();

    let synchronous: i64 = {
        let stmt = conn.prepare("PRAGMA synchronous").await.unwrap();
        let mut rows = stmt.query(()).await.unwrap();
        rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap()
    };
    assert_eq!(synchronous, 1, "synchronous must be NORMAL (1) after open");

    assert_eq!(
        scalar(&conn, "PRAGMA journal_mode").await.to_lowercase(),
        "wal",
        "journal_mode must be wal after open"
    );
}

#[tokio::test]
async fn fresh_open_then_reopen_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reopen.db");

    // Fresh open + one durable session through the Store trait.
    {
        let store = LibsqlStore::open(&path).await.unwrap();
        store
            .create_session(&SessionMeta {
                id: "boot-s1".into(),
                created_at: 1,
                updated_at: 1,
                ..Default::default()
            })
            .await
            .unwrap();
    }

    // Two re-opens on the same path: each re-runs the (single-transaction)
    // bootstrap. A nested BEGIN would fail the whole open loudly.
    for reopen in 1..=2 {
        let store = LibsqlStore::open(&path).await.unwrap();
        let conn = store.conn().await.unwrap();

        let session = store.get_session("boot-s1").await.unwrap();
        assert!(session.is_some(), "session must survive re-open #{reopen}");
        assert_eq!(
            count_schema_version_rows(&conn).await,
            1,
            "schema_version must hold exactly one row after re-open #{reopen}"
        );
        assert_eq!(
            scalar(&conn, "PRAGMA integrity_check").await,
            "ok",
            "integrity_check must be ok after re-open #{reopen}"
        );
    }
}

#[tokio::test]
async fn second_open_on_live_path_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("live.db");

    let first = LibsqlStore::open(&path).await.unwrap();
    let second = LibsqlStore::open(&path).await.unwrap();

    assert!(
        first.get_session("none").await.unwrap().is_none(),
        "baseline read on the first store"
    );
    second
        .create_session(&SessionMeta {
            id: "live-s1".into(),
            created_at: 1,
            updated_at: 1,
            ..Default::default()
        })
        .await
        .unwrap();
    let seen = first.get_session("live-s1").await.unwrap();
    assert!(seen.is_some(), "both handles must see the same database");

    let conn = first.conn().await.unwrap();
    assert_eq!(
        count_schema_version_rows(&conn).await,
        1,
        "double bootstrap must not duplicate the version row"
    );
}

#[tokio::test]
async fn legacy_tables_without_version_row_converge_on_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.db");
    seed_legacy_db(&path, false).await;

    // First open: previously failed inside the bootstrap transaction with
    // "no such column: task_type" (index over the stale sessions shape).
    let store = LibsqlStore::open(&path).await.unwrap();
    let conn = store.conn().await.unwrap();

    assert_eq!(
        version_of(&conn).await,
        32,
        "version row must be stamped at the latest version"
    );
    for (table, column) in [
        ("sessions", "task_type"),
        ("sessions", "handoff_seq"),
        ("sessions", "autopilot_mode"),
        ("sessions", "harness_runtime"),
        ("session_events", "sse_kind"),
        ("session_inputs", "images_json"),
        ("session_inputs", "recorded"),
    ] {
        assert!(
            has_column(&conn, table, column).await,
            "{table}.{column} must exist after the legacy repair"
        );
    }
    let stmt = conn
        .prepare("SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_sessions_task_type'")
        .await
        .unwrap();
    let mut rows = stmt.query(()).await.unwrap();
    assert_eq!(
        rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(),
        1,
        "idx_sessions_task_type must be built"
    );

    // Legacy data survives; the subagent child is backfilled, the parent keeps
    // the column default.
    let parent = store.get_session("parent").await.unwrap().unwrap();
    assert!(
        store.harness_runtime("parent").await.unwrap().is_none(),
        "legacy sessions have no external harness state"
    );
    assert_eq!(parent.task_type.as_deref(), Some(TASK_TYPE_PARENT));
    let child = store.get_session("child").await.unwrap().unwrap();
    assert_eq!(child.task_type.as_deref(), Some(TASK_TYPE_SUBAGENT));
    let stmt = conn.prepare("SELECT COUNT(*) FROM messages").await.unwrap();
    let mut rows = stmt.query(()).await.unwrap();
    assert_eq!(
        rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(),
        1
    );

    // Second open is idempotent: same version, no duplicate row, healthy db.
    drop(store);
    let store = LibsqlStore::open(&path).await.unwrap();
    let conn = store.conn().await.unwrap();
    assert_eq!(
        version_of(&conn).await,
        32,
        "re-open must not move the version"
    );
    assert_eq!(count_schema_version_rows(&conn).await, 1);
    assert!(has_column(&conn, "sessions", "task_type").await);
    assert_eq!(scalar(&conn, "PRAGMA integrity_check").await, "ok");
}

#[tokio::test]
async fn failed_bootstrap_rolls_back_and_reopens_after_repair() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rollback.db");
    seed_legacy_db(&path, true).await;

    let first = LibsqlStore::open(&path).await;
    assert!(
        first.is_err(),
        "index over the missing promoted_seq column must fail the open"
    );

    // Whole-transaction rollback: the db is exactly as hand-written. Nothing
    // the failed bootstrap created (schema_version, todo_workflows, ...)
    // survived, no migration column leaked, and the legacy row is untouched.
    let conn = raw_open(&path).await;
    assert!(
        !table_named(&conn, "schema_version").await,
        "the CREATE earlier in the tx must roll back"
    );
    assert!(
        !table_named(&conn, "todo_workflows").await,
        "mid-tx created tables must roll back"
    );
    assert!(
        !has_column(&conn, "sessions", "task_type").await,
        "no migration DDL may leak"
    );
    let stmt = conn.prepare("SELECT COUNT(*) FROM sessions").await.unwrap();
    let mut rows = stmt.query(()).await.unwrap();
    assert_eq!(
        rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(),
        2
    );

    // Repair the injected defect; the legacy path then converges normally.
    conn.execute(
        "ALTER TABLE session_inputs ADD COLUMN promoted_seq INTEGER",
        (),
    )
    .await
    .unwrap();
    drop(conn);

    let store = LibsqlStore::open(&path).await.unwrap();
    let conn = store.conn().await.unwrap();
    assert_eq!(version_of(&conn).await, 32);
    assert!(has_column(&conn, "sessions", "task_type").await);
    assert!(store.get_session("parent").await.unwrap().is_some());
    assert_eq!(scalar(&conn, "PRAGMA integrity_check").await, "ok");
}

#[tokio::test]
async fn checkpoint_gate_existing_path_reopen_converges() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gate.db");

    // Fresh file: gate input false - open skips the checkpoint.
    assert!(
        !path.exists(),
        "gate input must be false before the first open"
    );
    {
        let store = LibsqlStore::open(&path).await.unwrap();
        store
            .create_session(&SessionMeta {
                id: "gate-1".into(),
                created_at: 1,
                updated_at: 1,
                ..Default::default()
            })
            .await
            .unwrap();
    }
    // The file now pre-exists: the reopen below takes the checkpoint branch.
    assert!(path.exists(), "gate input must be true for the reopen");

    let store = LibsqlStore::open(&path).await.unwrap();
    let conn = store.conn().await.unwrap();
    assert!(store.get_session("gate-1").await.unwrap().is_some());
    assert_eq!(count_schema_version_rows(&conn).await, 1);
    assert_eq!(scalar(&conn, "PRAGMA integrity_check").await, "ok");
}
