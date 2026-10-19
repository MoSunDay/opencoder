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
use std::{
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
use tokio::sync::Notify;
use turso::Connection;

tokio::task_local! { pub(crate) static REQUEST_CONNECTION: Connection; }

#[derive(Clone)]
pub(crate) struct Database {
    pub db: opencoder_database::Database,
    #[cfg(test)]
    fallback: Connection,
    pub active: Arc<AtomicUsize>,
    pub changed: Arc<Notify>,
}

impl Database {
    pub async fn open(path: &Path, files: &Path) -> Result<Self, AppError> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let db = opencoder_database::Database::open(path).await?;
        let files = files.to_owned();
        db.write(move |connection| async move {
            schema::bootstrap(&connection, &files).await?;
            connection.execute("BEGIN IMMEDIATE", ()).await?;
            let store = Store {
                connection: connection.clone(),
                include_pending: false,
            };
            let result = store.initialize_debug().await;
            connection
                .execute(if result.is_ok() { "COMMIT" } else { "ROLLBACK" }, ())
                .await?;
            result?;
            Ok(())
        })
        .await?;
        Ok(Self {
            #[cfg(test)]
            fallback: db.connect().await?,
            db,
            active: Arc::new(AtomicUsize::new(0)),
            changed: Arc::new(Notify::new()),
        })
    }

    pub async fn store(&self) -> Store {
        #[cfg(test)]
        let connection = REQUEST_CONNECTION
            .try_with(Clone::clone)
            .unwrap_or_else(|_| self.fallback.clone());
        #[cfg(not(test))]
        let connection = REQUEST_CONNECTION
            .try_with(Clone::clone)
            .expect("ontology storage requires a leased request connection");
        Store {
            connection,
            include_pending: false,
        }
    }

    pub async fn ping(&self) -> Result<(), AppError> {
        self.db
            .read(|connection| async move {
                connection.query("SELECT 1", ()).await?.next().await?;
                Ok(())
            })
            .await?;
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
