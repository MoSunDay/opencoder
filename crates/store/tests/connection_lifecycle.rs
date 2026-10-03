//! Exercise the actual embedded backend's teardown, including late owners.

#[test]
fn embedded_connections_close_safely_across_fresh_runtimes_and_last_owners() {
    // Fresh runtimes and unreferenced connections reproduce libsql #2251.
    let iterations = if cfg!(windows) { 5_000 } else { 256 };
    for _ in 0..iterations {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let database = libsql::Builder::new_local(":memory:")
                .build()
                .await
                .unwrap();
            let connection = database.connect().unwrap();
            connection
                .execute("CREATE TABLE teardown_probe(value INTEGER)", ())
                .await
                .unwrap();
            drop(connection);

            let connection = database.connect().unwrap();
            let clone = connection.clone();
            let statement = connection.prepare("SELECT 42").await.unwrap();
            drop(connection);
            let transaction = clone.transaction().await.unwrap();
            transaction
                .execute("CREATE TABLE transaction_probe(value INTEGER)", ())
                .await
                .unwrap();
            drop(clone);
            transaction.commit().await.unwrap();
            let mut rows = statement.query(()).await.unwrap();
            drop(statement);
            let row = rows.next().await.unwrap().unwrap();
            assert_eq!(row.get::<i64>(0).unwrap(), 42);
            drop(rows);
            drop(row);
        });
    }
}

#[cfg(windows)]
#[tokio::test]
async fn last_store_connection_owner_releases_the_native_database_file() {
    use std::os::windows::fs::OpenOptionsExt;

    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("store.db");
    let store = opencoder_store::LibsqlStore::open(&path).await.unwrap();
    let connection = store.conn().await.unwrap();
    drop(store);
    let mut rows = connection.query("SELECT 1", ()).await.unwrap();
    assert_eq!(
        rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(),
        1
    );
    drop(rows);
    assert!(std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&path)
        .is_err());
    drop(connection);
    let exclusive = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(&path)
        .unwrap();
    drop(exclusive);
    std::fs::remove_file(&path).unwrap();
}
