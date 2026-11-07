//! Durable handoff state. All databases and advisory locks must be on local disk.
mod acceptance;
mod capacity;
mod inventory;
mod pending;
mod receipts;
mod runtimes;
pub use capacity::CapacitySnapshot;
pub use receipts::{dispatch_key, ExecutionNames, Receipt};
pub use runtimes::{RuntimeOwner, RuntimeRecord};

use super::FleetStore;
use anyhow::{Context, Result};
use std::fs::{File, OpenOptions};

impl FleetStore {
    /// A process lock covers a long operation; no engine transaction is held
    /// across a model call or RPC. The kernel releases it on process exit.
    /// Lock files must never be unlinked while any server can reference them.
    pub async fn request_lock(&self, scope: &str, id: &str) -> Result<File> {
        self.file_lock(scope, id, false).await
    }

    pub async fn shared_request_lock(&self, scope: &str, id: &str) -> Result<File> {
        self.file_lock(scope, id, true).await
    }

    async fn file_lock(&self, scope: &str, id: &str, shared: bool) -> Result<File> {
        let root = self
            .lock_dir
            .as_ref()
            .context("request locks require a local database")?;
        let name = opencoder_core::identity::token_hash(&format!("{scope}\0{id}"));
        let path = root.join(name);
        let file = tokio::task::spawn_blocking(move || {
            OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(path)
        })
        .await??;
        loop {
            let result = if shared {
                fs2::FileExt::try_lock_shared(&file)
            } else {
                fs2::FileExt::try_lock_exclusive(&file)
            };
            match result {
                Ok(()) => return Ok(file),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    // Cancellation drops the file immediately; a duplicate
                    // request never parks a blocking-pool thread indefinitely.
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
}
