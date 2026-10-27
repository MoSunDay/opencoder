//! Inspect supported binary resource containers without executing their code.
mod python;
mod sqlite;
mod wal;

pub(super) fn sidecar(path: &std::path::Path) -> anyhow::Result<bool> {
    wal::sidecar(path)
}

use anyhow::Result;
use serde_json::Value;

pub(super) async fn values(path: &std::path::Path, bytes: &[u8]) -> Result<Option<Vec<Value>>> {
    let strings = if bytes.starts_with(b"SQLite format 3\0") {
        Some(sqlite::strings(path, bytes).await?)
    } else if bytes.starts_with(&[0xa7, 0x0d, 0x0d, 0x0a]) {
        Some(python::strings(bytes)?)
    } else {
        None
    };
    Ok(strings.map(|strings| strings.into_iter().map(Value::String).collect()))
}
