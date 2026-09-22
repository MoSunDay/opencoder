//! One atomic report uses two SQL statements, independent of inventory size.
use anyhow::{bail, Result};
use libsql::Connection;

pub(super) async fn apply(conn: &Connection, encoded: &str) -> Result<()> {
    // Check every immutable field before changing any status. The caller owns
    // BEGIN IMMEDIATE, so another Server cannot alter ownership between these
    // statements. json_extract retains SQLite's signed 64-bit integer values.
    let mut conflicts = conn
        .query(
            "SELECT old.id FROM json_each(?1) AS incoming
             JOIN execution_index AS old ON old.id=json_extract(incoming.value,'$.id')
             WHERE old.created_at<>json_extract(incoming.value,'$.created_at')
                OR old.kind<>json_extract(incoming.value,'$.kind')
                OR old.node_id<>json_extract(incoming.value,'$.node_id')
             LIMIT 1",
            [encoded],
        )
        .await?;
    if let Some(row) = conflicts.next().await? {
        bail!("execution ownership conflict: {}", row.get::<String>(0)?);
    }
    drop(conflicts);
    conn.execute(
        "INSERT INTO execution_index(id,created_at,kind,node_id,status)
         SELECT json_extract(value,'$.id'),json_extract(value,'$.created_at'),
                json_extract(value,'$.kind'),json_extract(value,'$.node_id'),
                json_extract(value,'$.status')
         FROM json_each(?1) WHERE true
         ON CONFLICT(id) DO UPDATE SET status=excluded.status
         WHERE execution_index.status<>excluded.status",
        [encoded],
    )
    .await?;
    Ok(())
}
