//! Independent fleet control database: node catalog, five-column execution index,
//! and global definitions. It never opens an old session database.
use anyhow::Result;
use opencoder_database::Database;
use std::path::Path;
use turso::Connection;

mod brain;
mod catalog;
mod counts;
pub mod handoff;
mod records;
mod report;
mod schema;
#[derive(Clone)]
pub struct FleetStore {
    db: Database,
    lock_dir: Option<std::path::PathBuf>,
}

struct FleetConnection {
    conn: Connection,
}

impl FleetStore {
    pub async fn open(path: &Path) -> Result<Self> {
        let db = Database::open(path).await?;
        let mut store = Self::initialize(db).await?;
        let lock_dir = path.with_extension("locks");
        std::fs::create_dir_all(&lock_dir)?;
        store.lock_dir = Some(lock_dir);
        Ok(store)
    }
    pub async fn open_memory() -> Result<Self> {
        Self::initialize(Database::open_memory().await?).await
    }
    async fn initialize(db: Database) -> Result<Self> {
        db.write(|conn| async move {
            schema::initialize(&conn).await?;
            brain::initialize(&conn).await?;
            handoff::initialize(&conn).await?;
            Ok(())
        })
        .await?;
        Ok(Self { db, lock_dir: None })
    }
}
