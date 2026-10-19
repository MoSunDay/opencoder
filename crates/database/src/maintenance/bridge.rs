//! One native connection for maintenance clients that need a transaction.
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

async fn execute(connection: &turso::Connection, request: Value) -> Result<Value> {
    let sql = request["sql"].as_str().context("SQL text is required")?;
    let parameters = request["parameters"]
        .as_array()
        .context("SQL parameters must be an array")?
        .iter()
        .cloned()
        .map(super::parameter)
        .collect::<Result<Vec<_>>>()?;
    let mut result = connection
        .query(sql, turso::params_from_iter(parameters))
        .await?;
    let columns = result.column_names();
    let mut rows = Vec::new();
    while let Some(row) = result.next().await? {
        let mut cells = Vec::new();
        for index in 0..columns.len() {
            cells.push(match row.get_value(index)? {
                turso::Value::Null => Value::Null,
                turso::Value::Integer(value) => json!(value),
                turso::Value::Real(value) => json!(value),
                turso::Value::Text(value) => json!(value),
                turso::Value::Blob(value) => json!({"bytes": value}),
            });
        }
        rows.push(cells);
    }
    drop(result);
    let mut changes = connection
        .query("SELECT changes(), total_changes()", ())
        .await?;
    let changed = changes.next().await?.context("missing change counters")?;
    Ok(json!({"ok": true, "columns": columns, "rows": rows,
        "changes": changed.get::<i64>(0)?, "total_changes": changed.get::<i64>(1)?,
        "last_insert_rowid": connection.last_insert_rowid(), "autocommit": connection.is_autocommit()?}))
}

/// Read newline-delimited requests until EOF; unfinished transactions roll back.
pub async fn serve(path: PathBuf, read_only: bool) -> Result<()> {
    crate::runtime::run(true, async move {
        ensure!(path.is_file(), "database file does not exist");
        let database =
            turso::Builder::new_local(path.to_str().context("database path must be UTF-8")?)
                .experimental_multiprocess_wal(true)
                .read_only(read_only)
                .build()
                .await?;
        let connection = database.connect()?;
        connection.busy_timeout(std::time::Duration::from_secs(30))?;
        connection
            .execute_batch("PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;")
            .await?;
        if read_only {
            connection.execute("PRAGMA query_only=1", ()).await?;
        }
        let mut input = tokio::io::BufReader::new(tokio::io::stdin()).lines();
        let mut output = tokio::io::stdout();
        while let Some(line) = input.next_line().await? {
            let result = match serde_json::from_str(&line) {
                Ok(value) => execute(&connection, value).await,
                Err(error) => Err(error.into()),
            };
            let response = result.unwrap_or_else(|error| {
                json!({"ok": false, "error": error.to_string(),
                "autocommit": connection.is_autocommit().unwrap_or(false)})
            });
            output
                .write_all(serde_json::to_string(&response)?.as_bytes())
                .await?;
            output.write_all(b"\n").await?;
            output.flush().await?;
        }
        if !connection.is_autocommit()? {
            connection.execute("ROLLBACK", ()).await?;
        }
        Ok(())
    })
    .await
}
