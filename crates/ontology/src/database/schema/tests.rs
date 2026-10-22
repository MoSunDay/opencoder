use super::*;

async fn fixture() -> (opencoder_database::Database, Connection) {
    let db = opencoder_database::Database::open_memory().await.unwrap();
    let connection = db.connect().await.unwrap();
    bootstrap(
        &connection,
        Path::new("/test/files"),
        SchemaPolicy::InitializeEmpty,
    )
    .await
    .unwrap();
    (db, connection)
}

async fn legacy(connection: &Connection) {
    for name in ["catalog_entries", "catalog_objects", "catalog_state"] {
        connection
            .execute(&format!("DROP TABLE {name}"), ())
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn ordinary_open_rejects_old_schema_without_creating_tables() {
    let (_db, connection) = fixture().await;
    legacy(&connection).await;
    let before = objects(&connection).await.unwrap();
    for policy in [SchemaPolicy::InitializeEmpty, SchemaPolicy::Validate] {
        assert!(ready(&connection, Path::new("/test/files"), policy)
            .await
            .unwrap_err()
            .to_string()
            .contains("--maintenance"));
        assert_eq!(objects(&connection).await.unwrap(), before);
    }
    bootstrap(
        &connection,
        Path::new("/test/files"),
        SchemaPolicy::Maintenance,
    )
    .await
    .unwrap();
    assert!(ready(
        &connection,
        Path::new("/test/files"),
        SchemaPolicy::Validate
    )
    .await
    .unwrap());
    let after = objects(&connection).await.unwrap();
    bootstrap(
        &connection,
        Path::new("/test/files"),
        SchemaPolicy::Maintenance,
    )
    .await
    .unwrap();
    assert_eq!(objects(&connection).await.unwrap(), after);
}

#[tokio::test]
async fn valid_schema_checks_do_not_write_or_require_a_writer() {
    let (_db, connection) = fixture().await;
    connection.execute("PRAGMA query_only=1", ()).await.unwrap();
    assert!(ready(
        &connection,
        Path::new("/test/files"),
        SchemaPolicy::Validate
    )
    .await
    .unwrap());
    assert!(ready(
        &connection,
        Path::new("/different/files"),
        SchemaPolicy::Validate
    )
    .await
    .is_err());
}

#[tokio::test]
async fn conflicting_table_is_not_replaced_or_mistaken_for_complete() {
    let (_db, connection) = fixture().await;
    legacy(&connection).await;
    connection
        .execute(
            "CREATE TABLE catalog_entries(env_num INTEGER,id TEXT,body TEXT)",
            (),
        )
        .await
        .unwrap();
    let before = objects(&connection).await.unwrap();
    let error = bootstrap(
        &connection,
        Path::new("/test/files"),
        SchemaPolicy::Maintenance,
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("conflicts with catalog_entries"));
    assert_eq!(objects(&connection).await.unwrap(), before);
    assert!(connection.is_autocommit().unwrap());
}

#[tokio::test]
async fn missing_index_requires_maintenance_and_binding_is_never_rewritten() {
    let (_db, connection) = fixture().await;
    connection
        .execute("DROP INDEX entities_entity_type_id", ())
        .await
        .unwrap();
    assert!(ready(
        &connection,
        Path::new("/test/files"),
        SchemaPolicy::Validate
    )
    .await
    .is_err());
    assert!(bootstrap(
        &connection,
        Path::new("/changed"),
        SchemaPolicy::Maintenance
    )
    .await
    .is_err());
    assert!(!objects(&connection)
        .await
        .unwrap()
        .contains_key("entities_entity_type_id"));
    bootstrap(
        &connection,
        Path::new("/test/files"),
        SchemaPolicy::Maintenance,
    )
    .await
    .unwrap();
    assert!(ready(
        &connection,
        Path::new("/test/files"),
        SchemaPolicy::Validate
    )
    .await
    .unwrap());
}

#[tokio::test]
async fn failed_seed_rolls_back_all_new_tables() {
    let (_db, connection) = fixture().await;
    legacy(&connection).await;
    connection
        .execute("DELETE FROM environments", ())
        .await
        .unwrap();
    connection.execute("CREATE TRIGGER reject_seed BEFORE INSERT ON environments BEGIN SELECT RAISE(ABORT,'seed rejected'); END", ()).await.unwrap();
    let before = objects(&connection).await.unwrap();
    assert!(bootstrap(
        &connection,
        Path::new("/test/files"),
        SchemaPolicy::Maintenance
    )
    .await
    .is_err());
    assert_eq!(objects(&connection).await.unwrap(), before);
    connection
        .execute("DROP TRIGGER reject_seed", ())
        .await
        .unwrap();
    bootstrap(
        &connection,
        Path::new("/test/files"),
        SchemaPolicy::Maintenance,
    )
    .await
    .unwrap();
    assert!(ready(
        &connection,
        Path::new("/test/files"),
        SchemaPolicy::Validate
    )
    .await
    .unwrap());
}
