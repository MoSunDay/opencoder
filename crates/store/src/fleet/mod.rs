//! Independent fleet control database: node catalog, five-column execution index,
//! and global definitions. It never opens an old session database.
use anyhow::Result;
use libsql::{Builder, Connection, Database};
use std::path::Path;
use tokio::sync::Mutex;

mod records;
mod report;
mod schema;
pub struct FleetStore {
    _db: Database,
    pub(crate) conn: Connection,
    pub(crate) gate: Mutex<()>,
}

impl FleetStore {
    pub async fn open(path: &Path) -> Result<Self> {
        let db = Builder::new_local(path).build().await?;
        Self::initialize(db).await
    }
    pub async fn open_memory() -> Result<Self> {
        Self::initialize(Builder::new_local(":memory:").build().await?).await
    }
    async fn initialize(db: Database) -> Result<Self> {
        let conn = db.connect()?;
        conn.execute_batch("PRAGMA busy_timeout=30000; PRAGMA journal_mode=WAL;")
            .await?;
        schema::initialize(&conn).await?;
        Ok(Self {
            _db: db,
            conn,
            gate: Mutex::new(()),
        })
    }
}
