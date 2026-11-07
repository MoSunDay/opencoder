//! rdb (RocksDB) backend — the primary storage engine for the `Store`,
//! `ProjectStore`, `FleetStore` and `TsRegistry` seams. The ontology library
//! keeps its own SQL backend.
//!
//! ## Data model
//! Every logical row is one KV pair: the value is the JSON encoding of the
//! record; the key encodes domain, table, primary key and — for child rows —
//! the parent's primary key as a shared prefix, plus a zero-padded
//! monotonic ordinal where ordering matters (`msg/<session>/<seq:020>`).
//! Deletion of a parent removes the whole child range with
//! `delete_range`, which replaces SQL's `ON DELETE CASCADE`.
//!
//! ## Concurrency
//! One process-wide write lease per database (`engine::RdbDb::write`)
//! serializes writers exactly like `BEGIN IMMEDIATE`; each writer applies
//! all of its changes through a single atomic `batch_write`, so a failed
//! batch leaves nothing behind. Unique constraints are secondary index
//! keys (`idx/...`) maintained in the same batch and checked under the
//! lease; a hit maps to the same 409-style conflict the SQL backend
//! reports. Global monotonic cursors (`seq/<family>` counters persisted in
//! the same batch, holes allowed) back the `Last-Event-ID` replay streams.
//!
//! ## Layout
//! - `engine` — shared handle + write lease
//! - `kv`/`seq`/`index`/`tx`/`vector` — KV infrastructure
//! - `conversation` — sessions, inputs, messages, events, subagent tasks
//! - `todo` — todo workflows, items and transition events
//! - `execution` — brain, layered runs, nodes, node tasks, dag, schedules,
//!   team runs, reporting outboxes, inventory
//! - `identity` — users
//! - `projects` — `ProjectStore` implementation
pub mod engine;
pub mod events;

pub mod conversation;
pub mod execution;
pub mod identity;
pub mod index;
pub mod kv;
pub mod projects;
pub mod seq;
pub mod todo;
pub mod tx;
pub mod vector;

pub use engine::RdbDb;

// The single `impl Store for RdbStore` block, split into small includes.
mod impl_store;

/// RocksDB-backed `Store` implementation.
#[derive(Clone)]
pub struct RdbStore {
    pub(crate) db: RdbDb,
    /// Keeps a `open_memory()` temp directory alive for the store's life.
    _tmp: Option<std::sync::Arc<tempfile::TempDir>>,
}

impl RdbStore {
    /// Open the full runtime store (all domains).
    pub async fn open(path: impl AsRef<std::path::Path>) -> anyhow::Result<Self> {
        Ok(Self {
            db: RdbDb::open(path)?,
            _tmp: None,
        })
    }

    /// File-less store for tests: a temp-directory-backed real database.
    pub async fn open_memory() -> anyhow::Result<Self> {
        let dir = tempfile::tempdir()?;
        let db = RdbDb::open(dir.path())?;
        Ok(Self {
            db,
            _tmp: Some(std::sync::Arc::new(dir)),
        })
    }

    /// Shared Server definitions do not own Runtime inventory or report
    /// outboxes; this opener keeps that boundary by construction (the KV
    /// layout simply never installs those domains' data).
    pub async fn open_catalog(path: impl AsRef<std::path::Path>) -> anyhow::Result<Self> {
        Self::open(path).await
    }
}
