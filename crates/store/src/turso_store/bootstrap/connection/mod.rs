//! Bootstrap runs on the same isolated executor as later writes.
use super::schema;
use anyhow::Result;
use opencoder_database::Database;
use std::path::Path;

pub(super) async fn open_file(path: &Path) -> Result<Database> {
    let existed = path.exists();
    let started = std::time::Instant::now();
    let db = Database::open(path).await?;
    db.write(move |conn| async move {
        schema::apply_connection_pragmas(&conn).await?;
        schema::bootstrap(&conn).await?;
        if existed {
            schema::checkpoint_wal(&conn).await?;
        }
        Ok(())
    })
    .await?;
    tracing::info!(backend = "turso", path = %path.display(),
        total_ms = started.elapsed().as_millis() as u64, "store opened");
    Ok(db)
}
