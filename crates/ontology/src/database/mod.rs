mod entities;
pub(crate) mod model;
mod schema;
mod seeding;
mod store;
pub(crate) mod text_revision;
mod values;
mod vectors;

pub(crate) use store::Store;
pub(crate) use values::validate_structured_value;

use crate::error::AppError;
use libsql::{Builder, Connection};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
use tokio::sync::{Mutex, Notify};

tokio::task_local! { pub(crate) static REQUEST_CONNECTION: Connection; }

#[derive(Clone)]
pub(crate) struct Database {
    pub db: Arc<libsql::Database>,
    fallback: Connection,
    pub write_gate: Arc<Mutex<()>>,
    pub active: Arc<AtomicUsize>,
    pub changed: Arc<Notify>,
}

impl Database {
    pub async fn open(path: &Path, files: &Path) -> Result<Self, AppError> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let db = Arc::new(Builder::new_local(path).build().await?);
        let connection = db.connect()?;
        let mut rows = connection.query("PRAGMA journal_mode=WAL", ()).await?;
        while rows.next().await?.is_some() {}
        connection.execute("PRAGMA foreign_keys=ON", ()).await?;
        connection.busy_timeout(std::time::Duration::from_secs(30))?;
        schema::bootstrap(&connection, files).await?;
        let this = Self {
            db,
            fallback: connection,
            write_gate: Arc::new(Mutex::new(())),
            active: Arc::new(AtomicUsize::new(0)),
            changed: Arc::new(Notify::new()),
        };
        let store = this.store().await;
        store.connection.execute("BEGIN IMMEDIATE", ()).await?;
        let result = store.initialize_debug().await;
        store
            .connection
            .execute(if result.is_ok() { "COMMIT" } else { "ROLLBACK" }, ())
            .await?;
        result?;
        Ok(this)
    }

    pub async fn store(&self) -> Store {
        let connection = REQUEST_CONNECTION
            .try_with(Clone::clone)
            .unwrap_or_else(|_| self.fallback.clone());
        Store {
            connection,
            include_pending: false,
        }
    }

    pub async fn ping(&self) -> Result<(), AppError> {
        self.db.connect()?.query("SELECT 1", ()).await?;
        Ok(())
    }

    pub async fn drained(&self) {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.active.load(Ordering::SeqCst) == 0 {
                return;
            }
            changed.await;
        }
    }
}
