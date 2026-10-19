//! Inspect supported binary resource containers without executing their code.
mod python;
mod sqlite;
mod wal;

pub(super) fn sidecar(path: &std::path::Path) -> anyhow::Result<bool> {
    wal::sidecar(path)
}

use anyhow::Result;
use serde_json::Value;

pub(super) fn values(path: &std::path::Path, bytes: &[u8]) -> Result<Option<Vec<Value>>> {
    let strings = if bytes.starts_with(b"SQLite format 3\0") {
        Some(tokio::runtime::Handle::current().block_on(sqlite::strings(path, bytes))?)
    } else if bytes.starts_with(&[0xa7, 0x0d, 0x0d, 0x0a]) {
        Some(python::strings(bytes)?)
    } else {
        None
    };
    Ok(strings.map(|strings| strings.into_iter().map(Value::String).collect()))
}
