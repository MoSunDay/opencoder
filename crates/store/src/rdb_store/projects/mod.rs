//! Project-module persistence over the KV engine, one row family per file.
//!
//! The layout maps one logical SQL row to one KV pair (see [`keys`] for the families and index keys); every multi-row
//! mutation stages all rows, index entries and counters in ONE atomic
//! `WriteBatch`, so a failure mid-build leaves nothing behind — the KV
//! equivalent of a rolled-back `BEGIN IMMEDIATE` transaction. Free
//! functions take `&rdb::store::Store`; [`implementation`] holds only the
//! thin `ProjectStore for RdbStore` delegations.

pub(crate) mod acceptance;
pub(crate) mod comments;
pub(crate) mod goals;
mod implementation;
pub(crate) mod initiatives;
pub(crate) mod keys;
pub(crate) mod links;
pub(crate) mod people;
pub(crate) mod runs;
pub(crate) mod runs_inspect;
pub(crate) mod summary;
pub(crate) mod tags;
pub(crate) mod todos;
