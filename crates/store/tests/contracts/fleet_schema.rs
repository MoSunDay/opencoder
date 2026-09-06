use libsql::Builder;
use opencoder_core::fleet::ExecutionKind;
use opencoder_store::fleet::FleetStore;

const LEGACY_SCHEMA: &str = "CREATE TABLE execution_index (
    id TEXT PRIMARY KEY, created_at INTEGER NOT NULL,
    node_id TEXT NOT NULL, status TEXT NOT NULL)";

async fn seed(path: &std::path::Path, id: &str) {
    seed_with_status(path, id, "running").await;
}

async fn seed_with_status(path: &std::path::Path, id: &str, status: &str) {
    let db = Builder::new_local(path).build().await.unwrap();
    let conn = db.connect().unwrap();
    conn.execute(LEGACY_SCHEMA, ()).await.unwrap();
    conn.execute(
        "INSERT INTO execution_index VALUES (?1,1,'node-a',?2)",
        [id, status],
    )
    .await
    .unwrap();
}

async fn backup_exists(path: &std::path::Path) -> bool {
    let db = Builder::new_local(path).build().await.unwrap();
    let conn = db.connect().unwrap();
    let mut rows = conn
        .query(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='execution_index_v2_backup'",
            (),
        )
        .await
        .unwrap();
    rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap() != 0
}

async fn table_count(path: &std::path::Path, table: &str) -> i64 {
    let db = Builder::new_local(path).build().await.unwrap();
    let conn = db.connect().unwrap();
    let mut rows = conn
        .query(&format!("SELECT count(*) FROM {table}"), ())
        .await
        .unwrap();
    rows.next().await.unwrap().unwrap().get(0).unwrap()
}

#[tokio::test]
async fn exact_v2_schema_migrates_kind_and_retains_backup_table() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("control.db");
    seed(&path, "prun-old").await;

    let store = FleetStore::open(&path).await.unwrap();
    let record = store.index("prun-old").await.unwrap().unwrap();
    assert_eq!(record.kind, ExecutionKind::Project);
    drop(store);

    assert_eq!(table_count(&path, "execution_index").await, 1);
    assert_eq!(table_count(&path, "execution_index_v2_backup").await, 1);
}

#[tokio::test]
async fn unknown_legacy_prefix_rolls_back_without_replacing_table() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("control.db");
    seed(&path, "unknown-old").await;

    let error = FleetStore::open(&path).await.err().unwrap().to_string();
    assert!(error.contains("no recognized kind prefix"), "{error}");
    assert_eq!(table_count(&path, "execution_index").await, 1);

    assert!(!backup_exists(&path).await);
}

#[tokio::test]
async fn unknown_legacy_status_rolls_back_without_replacing_table() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("control.db");
    seed_with_status(&path, "agent-old", "unknown").await;

    let error = FleetStore::open(&path).await.err().unwrap().to_string();
    assert!(error.contains("invalid status"), "{error}");
    assert_eq!(table_count(&path, "execution_index").await, 1);
    assert!(!backup_exists(&path).await);
}

#[tokio::test]
async fn near_match_schema_is_rejected_instead_of_stamped_current() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("control.db");
    let db = Builder::new_local(&path).build().await.unwrap();
    let conn = db.connect().unwrap();
    conn.execute(
        "CREATE TABLE execution_index (
            id TEXT PRIMARY KEY, created_at INTEGER NOT NULL,
            kind TEXT, node_id TEXT NOT NULL, status TEXT NOT NULL)",
        (),
    )
    .await
    .unwrap();
    drop(conn);
    drop(db);

    let error = FleetStore::open(&path).await.err().unwrap().to_string();
    assert!(
        error.contains("unsupported execution_index schema"),
        "{error}"
    );
}

#[tokio::test]
async fn current_schema_with_unknown_kind_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("control.db");
    let db = Builder::new_local(&path).build().await.unwrap();
    let conn = db.connect().unwrap();
    conn.execute(
        "CREATE TABLE execution_index (
            id TEXT PRIMARY KEY, created_at INTEGER NOT NULL, kind TEXT NOT NULL,
            node_id TEXT NOT NULL, status TEXT NOT NULL)",
        (),
    )
    .await
    .unwrap();
    conn.execute(
        "INSERT INTO execution_index VALUES ('agent-bad',1,'unknown','node-a','running')",
        (),
    )
    .await
    .unwrap();
    drop(conn);
    drop(db);

    let error = FleetStore::open(&path).await.err().unwrap().to_string();
    assert!(error.contains("invalid kind"), "{error}");
}

#[tokio::test]
async fn current_schema_with_unknown_status_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("control.db");
    let db = Builder::new_local(&path).build().await.unwrap();
    let conn = db.connect().unwrap();
    conn.execute(
        "CREATE TABLE execution_index (
            id TEXT PRIMARY KEY, created_at INTEGER NOT NULL, kind TEXT NOT NULL,
            node_id TEXT NOT NULL, status TEXT NOT NULL)",
        (),
    )
    .await
    .unwrap();
    conn.execute(
        "INSERT INTO execution_index VALUES ('agent-bad',1,'agent','node-a','unknown')",
        (),
    )
    .await
    .unwrap();
    drop(conn);
    drop(db);

    let error = FleetStore::open(&path).await.err().unwrap().to_string();
    assert!(error.contains("invalid status"), "{error}");
}
