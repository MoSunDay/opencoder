use std::path::Path;

use anyhow::Result;
use libsql::Connection;
use tokio::sync::Mutex;

mod brain;
mod brain_playbooks;
mod chat_tables;
mod connection;
mod dag;
mod dag_events;
mod events;
mod impl_store;
mod inputs;
mod messages;
mod node_state;
mod node_tasks;
mod nodes;
mod project;
mod project_runs;
pub(crate) mod schema;
mod sessions;
mod subagent_tasks;
mod team_runs;
mod todos;
mod tx;
mod users;

/// Primary `Store` implementation backed by libsql (embedded local SQLite, WAL).
///
/// Holds ONE connection obtained at open time; each operation clones it. libsql
/// connection clones share the same underlying database — this makes in-memory
/// databases work correctly across operations (a fresh `db.connect()` per op
/// would hand back an empty `:memory:` db every time) while file-backed dbs
/// still get WAL semantics. All SQL lives in free functions in the submodules
/// so the backend can be swapped without touching callers.
pub struct LibsqlStore {
    conn: Connection,
    /// Serializes all DB operations. libsql 0.9.x local backend runs sync
    /// SQLite FFI directly on the tokio worker thread; without serialization,
    /// concurrent operations (multi-subagent flushers + run_loop) contend on
    /// SQLite's internal mutex, starving the runtime. An async Mutex yields on
    /// contention (never blocks a worker thread) while ensuring at most one
    /// worker touches SQLite FFI at a time.
    db_lock: Mutex<()>,
}

impl LibsqlStore {
    /// Open (or create) a libsql database file and bootstrap the schema.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            conn: connection::open_file(path.as_ref()).await?,
            db_lock: Mutex::new(()),
        })
    }

    /// Open an in-memory database (used by tests and ephemeral runs).
    pub async fn open_memory() -> Result<Self> {
        Ok(Self {
            conn: connection::open_memory().await?,
            db_lock: Mutex::new(()),
        })
    }

    /// Acquire a connection that shares the underlying database. Cheap clone.
    pub async fn conn(&self) -> Result<Connection> {
        Ok(self.conn.clone())
    }
}
