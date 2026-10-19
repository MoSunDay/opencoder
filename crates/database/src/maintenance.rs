//! Read-only inspection and consistent online copies using the owning engine.
use crate::runtime;
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
mod bridge;
pub use bridge::serve;

async fn reader(path: &Path) -> Result<(turso::Database, turso::Connection)> {
    ensure!(
        path.is_file(),
        "database file does not exist: {}",
        path.display()
    );
    let database = turso::Builder::new_local(path.to_str().context("database path must be UTF-8")?)
        .experimental_multiprocess_wal(true)
        .read_only(true)
        .build()
        .await?;
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
pub async fn checkpoint(path: PathBuf) -> Result<()> {
    runtime::run(true, async move {
        ensure!(path.is_file(), "database file does not exist");
        let database =
            turso::Builder::new_local(path.to_str().context("database path must be UTF-8")?)
                .experimental_multiprocess_wal(true)
                .build()
                .await?;
        let connection = database.connect()?;
        connection.busy_timeout(std::time::Duration::from_secs(30))?;
        let mut rows = connection
            .query("PRAGMA wal_checkpoint(TRUNCATE)", ())
            .await?;
        while let Some(row) = rows.next().await? {
            ensure!(row.get::<i64>(0)? == 0, "database checkpoint is busy");
        }
        Ok(())
    })
    .await
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
        // Turso 0.8.3 does not unescape VACUUM literals. On Linux, address a
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
