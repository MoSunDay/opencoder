use opencoder_database::{maintenance, Database};
use serde_json::json;

#[tokio::test]
async fn inspection_rejects_writes_and_backup_preserves_a_live_database() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.db");
    let output = dir.path().join("copy.db");
    let db = Database::open(&source).await.unwrap();
    db.write(|conn| async move {
        conn.execute_batch("CREATE TABLE items(id INTEGER PRIMARY KEY, value TEXT); INSERT INTO items VALUES(1,'before');").await?;
        Ok(())
    }).await.unwrap();
    let before = maintenance::query(
        source.clone(),
        "SELECT value FROM items WHERE id=?1".into(),
        vec![json!(1)],
    )
    .await
    .unwrap();
    assert_eq!(before["rows"], json!([["before"]]));
    assert!(
        maintenance::query(source.clone(), "DELETE FROM items".into(), vec![])
            .await
            .is_err()
    );
    maintenance::backup(source.clone(), output.clone())
        .await
        .unwrap();
    db.write(|conn| async move {
        conn.execute("UPDATE items SET value='after'", ()).await?;
        Ok(())
    })
    .await
    .unwrap();
    let saved = maintenance::query(output.clone(), "SELECT value FROM items".into(), vec![])
        .await
        .unwrap();
    assert_eq!(saved["rows"], json!([["before"]]));
    assert!(maintenance::backup(source, output).await.is_err());
}
