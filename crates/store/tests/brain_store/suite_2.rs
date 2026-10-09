use super::*;

#[tokio::test]
async fn migration_v14_to_v15_creates_brain_tables() {
    let dir: TempDir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("brain-migrate.db");
    {
        let db = libsql::Builder::new_local(&db_path).build().await.unwrap();
        let conn = db.connect().unwrap();
        conn.execute("CREATE TABLE schema_version (version INTEGER NOT NULL)", ())
            .await
            .unwrap();
        conn.execute("INSERT INTO schema_version (version) VALUES (14)", ())
            .await
            .unwrap();
    }

    let store = LibsqlStore::open(&db_path).await.unwrap();

    // Schema version bumped to the latest (34).
    assert_eq!(
        scalar_i64(&store, "SELECT version FROM schema_version LIMIT 1").await,
        34
    );

    // All three brain tables now exist.
    for table in ["brain_capabilities", "brain_eng_inputs", "brain_vectors"] {
        assert_eq!(
            scalar_i64(
                &store,
                &format!(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='{table}'"
                )
            )
            .await,
            1,
            "{table} must exist after v14→v15 migration"
        );
    }

    // The migrated schema is immediately usable end-to-end.
    store
        .create_brain_capability(&cap("cap-m", "goal", 1), &[eng("cap-m", "in", 0)])
        .await
        .unwrap();
    store
        .upsert_brain_vector("cap-m", 4, "emb", &le(&[1.0, 0.0, 0.0, 0.0]), 1)
        .await
        .unwrap();
    let hits = store
        .search_brain_vectors("emb", &le(&[1.0, 0.0, 0.0, 0.0]), 5)
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].capability.id, "cap-m");
}
