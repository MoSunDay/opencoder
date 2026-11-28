//! Read-only inspection and consistent online copies using the owning engine.
use crate::runtime;
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
mod bridge;
pub use bridge::serve;

/// Open `path` read-only through the file it resolves to.
///
/// The engine derives the WAL and shared-memory companions from the path it
/// is given, so inspection must open the canonical file: through a symlink it
/// would otherwise look for a WAL companion that never belonged to the
/// database and report an empty or stale snapshot.
async fn reader(path: &Path) -> Result<(turso::Database, turso::Connection)> {
    ensure!(
        path.is_file(),
        "database file does not exist: {}",
        path.display()
    );
    let canonical = path.canonicalize()?;
    let path = canonical.as_path();
    let database = crate::engine::open(crate::engine::engine_name(path)?, true, path).await?;
    let connection = database.connect()?;
    connection.busy_timeout(std::time::Duration::from_secs(30))?;
    Ok((database, connection))
}

/// Materialize a read-only query; inspection never changes application records.
pub async fn query(path: PathBuf, sql: String, parameters: Vec<Value>) -> Result<Value> {
    runtime::run(false, async move {
        let (_database, connection) = reader(&path).await?;
        connection.execute("PRAGMA query_only=1", ()).await?;
        let parameters = parameters
            .into_iter()
            .map(parameter)
            .collect::<Result<Vec<_>>>()?;
        let mut rows = connection
            .query(sql, turso::params_from_iter(parameters))
            .await?;
        let columns = rows.column_names();
        let mut values = Vec::new();
        while let Some(row) = rows.next().await? {
            let mut cells = Vec::with_capacity(columns.len());
            for index in 0..columns.len() {
                cells.push(match row.get_value(index)? {
                    turso::Value::Null => Value::Null,
                    turso::Value::Integer(value) => json!(value),
                    turso::Value::Real(value) => json!(value),
                    turso::Value::Text(value) => json!(value),
                    turso::Value::Blob(value) => json!({"bytes": value}),
                });
            }
            values.push(cells);
        }
        Ok(json!({"columns": columns, "rows": values}))
    })
    .await
}

fn parameter(value: Value) -> Result<turso::Value> {
    Ok(match value {
        Value::Null => turso::Value::Null,
        Value::Bool(value) => turso::Value::Integer(i64::from(value)),
        Value::Number(value) => match value.as_i64() {
            Some(value) => turso::Value::Integer(value),
            None => {
                ensure!(!value.is_u64(), "SQL integer exceeds signed 64-bit range");
                turso::Value::Real(value.as_f64().context("invalid SQL number")?)
            }
        },
        Value::String(value) => turso::Value::Text(value),
        Value::Object(value) if value.len() == 1 && value.contains_key("bytes") => {
            turso::Value::Blob(serde_json::from_value(value["bytes"].clone())?)
        }
        _ => anyhow::bail!("SQL parameters must be JSON scalars"),
    })
}

/// Called after the deployment controller has stopped every writer.
///
/// TRUNCATE rewrites the WAL in place, so it takes the database's exclusive
/// maintenance lease on `<db>.maintenance.lock` -- the file every live
/// writer pool holds a shared lease on -- and keeps it until the file is
/// self-contained, which also keeps new writers out meanwhile. While any
/// writer is alive the call fails fast with "writers still active" instead of
/// contending with it; a reader still holding the WAL makes the engine report
/// busy. The engine opens the canonical path, the one the lease was taken
/// for, so the WAL it truncates is the WAL that belongs to the leased file
/// even when `path` is a symlink.
pub async fn checkpoint(path: PathBuf) -> Result<()> {
    runtime::run(true, async move {
        ensure!(
            path.is_file(),
            "database file does not exist: {}",
            path.display()
        );
        let key = path
            .canonicalize()
            .with_context(|| format!("resolve database {}", path.display()))?;
        // Refuse a path the engine cannot open before taking the lease, so a
        // refused TRUNCATE leaves no lease file behind.
        let name = crate::engine::engine_name(&key)?;
        let _lease = crate::maintenance_lock::try_acquire_exclusive(&key)
            .map_err(|error| writers_still_active(&key, error))?;
        let database = crate::engine::open(name, false, &key).await?;
        let connection = database.connect()?;
        connection.busy_timeout(std::time::Duration::from_secs(30))?;
        let mut rows = connection
            .query("PRAGMA wal_checkpoint(TRUNCATE)", ())
            .await?;
        while let Some(row) = rows.next().await? {
            ensure!(
                row.get::<i64>(0)? == 0,
                "readers/writers still active: the database checkpoint is busy; let every \
                 reader finish and stop every writer before a TRUNCATE checkpoint"
            );
        }
        Ok(())
    })
    .await
}

/// Only a real lease conflict means "writers still active". Every other
/// failure -- permissions, a missing or unsupported lock, an I/O error --
/// keeps its own context, so the operator sees the actual cause instead of
/// stopping writers that were never running.
fn writers_still_active(database: &Path, error: anyhow::Error) -> anyhow::Error {
    let conflict = crate::maintenance_lock::is_writers_active(&error)
        || error
            .chain()
            .filter_map(|cause| cause.downcast_ref::<std::io::Error>())
            .any(|cause| cause.kind() == std::io::ErrorKind::WouldBlock);
    if conflict {
        // Context keeps the lease conflict in the chain, so a caller can still
        // branch on `is_writers_active` after the operator-facing message.
        error.context(format!(
            "writers still active: stop every writer of {} before a TRUNCATE checkpoint",
            database.display()
        ))
    } else {
        error
    }
}

/// VACUUM INTO owns one database snapshot. Different files are independent copies.
pub async fn backup(path: PathBuf, output: PathBuf) -> Result<()> {
    runtime::run(false, async move {
        ensure!(!output.exists(), "backup output already exists");
        if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let parent = output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let temporary = tempfile::Builder::new()
            .prefix(".turso-backup-")
            .tempdir_in(parent)?;
        let snapshot = temporary.path().join("snapshot.db");
        let (database, connection) = reader(&path).await?;
        // Turso does not unescape VACUUM literals. On Linux, address a
        // private staging directory by fd so arbitrary parent names stay safe.
        #[cfg(target_os = "linux")]
        let directory = std::fs::File::open(temporary.path())?;
        #[cfg(target_os = "linux")]
        let destination = {
            use std::os::fd::AsRawFd;
            format!("/proc/self/fd/{}/snapshot.db", directory.as_raw_fd())
        };
        #[cfg(not(target_os = "linux"))]
        let destination = snapshot
            .to_str()
            .context("backup path must be UTF-8")?
            .to_owned();
        ensure!(
            !destination.contains(['\0', '\'']),
            "unsupported VACUUM directory path"
        );
        connection
            .execute(format!("VACUUM INTO '{destination}'"), ())
            .await?;
        drop(connection);
        drop(database);
        let (copy, connection) = reader(&snapshot).await?;
        let mut rows = connection.query("PRAGMA quick_check", ()).await?;
        let mut checked = false;
        while let Some(row) = rows.next().await? {
            ensure!(
                row.get::<String>(0)? == "ok",
                "backup integrity check failed"
            );
            checked = true;
        }
        ensure!(checked, "backup integrity check returned no result");
        drop(rows);
        drop(connection);
        drop(copy);
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&snapshot)?
            .sync_all()?;
        // Atomic publication must not replace an existing file or symlink.
        std::fs::hard_link(&snapshot, &output)?;
        #[cfg(unix)]
        if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::File::open(parent)?.sync_all()?;
        }
        Ok(())
    })
    .await
}
