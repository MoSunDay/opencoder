use crate::error::AppError;
use libsql::Connection;

pub(super) const OBJECT_TABLES: &[&str] = &[
    "entity_types",
    "attribute_definitions",
    "entity_type_actions",
    "entities",
    "relationship_types",
    "relationships",
    "graph_aspects",
    "structured_values",
    "text_revisions",
    "audit_events",
];

pub(super) async fn bootstrap(
    connection: &Connection,
    files: &std::path::Path,
) -> Result<(), AppError> {
    connection.execute("BEGIN IMMEDIATE", ()).await?;
    let result = apply(connection, files).await;
    connection
        .execute(if result.is_ok() { "COMMIT" } else { "ROLLBACK" }, ())
        .await?;
    result
}

async fn apply(connection: &Connection, files: &std::path::Path) -> Result<(), AppError> {
    connection.execute("CREATE TABLE IF NOT EXISTS ontology_schema_version(version INTEGER PRIMARY KEY CHECK(version=1),files_root TEXT NOT NULL)", ()).await?;
    bind_files_root(connection, files).await?;
    connection.execute("CREATE TABLE IF NOT EXISTS environments(env_num INTEGER PRIMARY KEY,env_key TEXT NOT NULL UNIQUE,body TEXT NOT NULL CHECK(json_valid(body)))", ()).await?;
    for table in OBJECT_TABLES {
        connection.execute(&format!("CREATE TABLE IF NOT EXISTS {table}(env_num INTEGER NOT NULL,id TEXT NOT NULL,body TEXT NOT NULL CHECK(json_valid(body)),PRIMARY KEY(env_num,id))"), ()).await?;
    }
    for (table, field) in [
        ("entity_types", "type_key"),
        ("relationship_types", "type_key"),
        ("graph_aspects", "aspect_key"),
    ] {
        connection.execute(&format!("CREATE UNIQUE INDEX IF NOT EXISTS {table}_key ON {table}(env_num,json_extract(body,'$.{field}'))"), ()).await?;
    }
    connection.execute("CREATE UNIQUE INDEX IF NOT EXISTS attribute_key ON attribute_definitions(env_num,json_extract(body,'$.entity_type_id'),json_extract(body,'$.attribute_key'))", ()).await?;
    for (table, field) in [
        ("entities", "entity_type_id"),
        ("relationships", "source_entity_id"),
        ("relationships", "target_entity_id"),
        ("structured_values", "entity_id"),
        ("text_revisions", "entity_id"),
    ] {
        connection.execute(&format!("CREATE INDEX IF NOT EXISTS {table}_{field} ON {table}(env_num,json_extract(body,'$.{field}'))"), ()).await?;
    }
    connection.execute("CREATE TABLE IF NOT EXISTS vectors(env_num INTEGER NOT NULL,attribute_id TEXT NOT NULL,entity_id TEXT NOT NULL,vector_id TEXT NOT NULL,revision INTEGER NOT NULL,is_deleted INTEGER NOT NULL,emb BLOB NOT NULL,PRIMARY KEY(env_num,attribute_id,entity_id),UNIQUE(env_num,vector_id))", ()).await?;
    Ok(())
}

async fn bind_files_root(connection: &Connection, files: &std::path::Path) -> Result<(), AppError> {
    let requested = files.to_string_lossy().to_string();
    let mut rows = connection
        .query(
            "SELECT files_root FROM ontology_schema_version WHERE version=1",
            (),
        )
        .await?;
    if let Some(row) = rows.next().await? {
        if row.get::<String>(0)? != requested {
            // Other Server versions can still use this database. A restart must
            // never silently redirect their current or historical text files.
            return Err(AppError::config(
                "ontology files directory differs from the database binding; retain the original directory and restore the database and text files together",
            ));
        }
        return Ok(());
    }
    connection
        .execute(
            "INSERT INTO ontology_schema_version VALUES(1,?1)",
            [requested],
        )
        .await?;
    Ok(())
}
