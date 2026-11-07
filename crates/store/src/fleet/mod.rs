//! Independent fleet control database: node catalog, five-column execution index,
//! global definitions and handoff state. It never opens a session database.
use anyhow::Result;
use std::path::Path;

mod brain;
mod catalog;
mod counts;
pub mod handoff;
mod inventory_meta;
mod keys;
mod ownership;
mod records;
mod report;
pub use report::IndexReportBaselineChanged;
mod schema;

use crate::rdb_store::engine::RdbDb;

#[derive(Clone)]
pub struct FleetStore {
    db: RdbDb,
    lock_dir: Option<std::path::PathBuf>,
    /// Keeps an `open_memory()` temp directory alive for the store's life.
    _tmp: Option<std::sync::Arc<tempfile::TempDir>>,
}

impl FleetStore {
    /// RocksDB needs a directory, so `<path>.rdb` is appended (the legacy file
    /// name is preserved as the directory stem, e.g. `/x/host.db.rdb`).
    pub async fn open(path: &Path) -> Result<Self> {
        let dir = format!("{}.rdb", path.display());
        if let Some(parent) = Path::new(&dir).parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut store = Self::initialize(RdbDb::open(&dir)?).await?;
        let lock_dir = path.with_extension("locks");
        std::fs::create_dir_all(&lock_dir)?;
        store.lock_dir = Some(lock_dir);
        Ok(store)
    }
    pub async fn open_memory() -> Result<Self> {
        let dir = tempfile::tempdir()?;
        let db = RdbDb::open(dir.path())?;
        let store = Self::initialize(db).await?;
        Ok(Self {
            db: store.db,
            lock_dir: None,
            _tmp: Some(std::sync::Arc::new(dir)),
        })
    }
    async fn initialize(db: RdbDb) -> Result<Self> {
        db.write(schema::initialize).await?;
        Ok(Self {
            db,
            lock_dir: None,
            _tmp: None,
        })
    }
}
