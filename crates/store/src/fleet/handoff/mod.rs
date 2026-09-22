//! Durable handoff state. All databases and advisory locks must be on local disk.
mod capacity;
#[cfg(test)]
mod capacity_tests;
mod inventory;
mod pending;
mod receipts;
mod runtimes;
pub use capacity::CapacitySnapshot;
pub use receipts::{dispatch_key, ExecutionNames, Receipt};
pub use runtimes::{RuntimeOwner, RuntimeRecord};

use super::FleetStore;
use anyhow::{Context, Result};
use libsql::Connection;
use std::fs::{File, OpenOptions};

pub(super) async fn initialize(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS dispatch_receipts (
            scope TEXT NOT NULL, id TEXT NOT NULL, fingerprint TEXT NOT NULL,
            phase TEXT NOT NULL, payload TEXT NOT NULL, PRIMARY KEY(scope,id));
         CREATE TABLE IF NOT EXISTS execution_assignments (
            id TEXT PRIMARY KEY, assignment TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS node_report_watermarks (
            node_id TEXT PRIMARY KEY, generation TEXT NOT NULL, sequence INTEGER NOT NULL);
         CREATE TABLE IF NOT EXISTS host_runtimes (
            id TEXT PRIMARY KEY, release_id TEXT NOT NULL, config TEXT NOT NULL, mode TEXT NOT NULL);
         CREATE UNIQUE INDEX IF NOT EXISTS one_active_runtime ON host_runtimes(mode) WHERE mode='active';
         CREATE TABLE IF NOT EXISTS runtime_owners (
            execution_id TEXT PRIMARY KEY, runtime_id TEXT NOT NULL REFERENCES host_runtimes(id));
         CREATE TABLE IF NOT EXISTS host_capacity (
            singleton INTEGER PRIMARY KEY CHECK(singleton=1), max_runs INTEGER NOT NULL CHECK(max_runs>0));
         CREATE TABLE IF NOT EXISTS capacity_queue (
            sequence INTEGER PRIMARY KEY AUTOINCREMENT, ticket TEXT UNIQUE NOT NULL,
            execution_id TEXT NOT NULL, runtime_id TEXT NOT NULL,
            phase TEXT NOT NULL CHECK(phase IN ('queued','running','done')));
         CREATE UNIQUE INDEX IF NOT EXISTS one_live_slot ON capacity_queue(execution_id) WHERE phase!='done';",
    ).await?;
    Ok(())
}

impl FleetStore {
    /// A process lock covers a long operation; no SQLite transaction is held
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
