//! One native connection for maintenance clients that need a transaction.
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

/// How long an open write transaction may outlive the last input line before
/// the watchdog rolls it back. An unbounded hold blocks every other process
/// writing the same database until their busy timeout expires.
const IDLE_WRITE_TX_LIMIT: Duration = Duration::from_secs(30);

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

/// Read newline-delimited requests from stdin; unfinished transactions roll back.
pub async fn serve(path: PathBuf, read_only: bool) -> Result<()> {
    let input = tokio::io::BufReader::new(tokio::io::stdin());
    let output = tokio::io::stdout();
    crate::runtime::run(true, serve_streams(path, read_only, IDLE_WRITE_TX_LIMIT, input, output))
        .await
}

/// The bridge body over generic streams so tests can drive the watchdog.
///
/// While a request leaves a write transaction open, the next line read
/// carries a deadline: a client stalling mid-transaction cannot pin the
/// database's write lock indefinitely, so the watchdog rolls the transaction
/// back and keeps serving. Autocommit and read-only states never hold the
/// write lock and may wait for input without a deadline.
async fn serve_streams<R, W>(
    path: PathBuf,
    read_only: bool,
    idle_write_tx_limit: Duration,
    input: tokio::io::BufReader<R>,
    mut output: W,
) -> Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    ensure!(path.is_file(), "database file does not exist");
    let database =
        turso::Builder::new_local(path.to_str().context("database path must be UTF-8")?)
            .experimental_multiprocess_wal(true)
            .read_only(read_only)
            .build()
            .await?;
    let connection = database.connect()?;
    connection.busy_timeout(Duration::from_secs(30))?;
    connection
        .execute_batch("PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;")
        .await?;
    if read_only {
        connection.execute("PRAGMA query_only=1", ()).await?;
    }
    let mut input = input.lines();
    loop {
        let line = if read_only || connection.is_autocommit()? {
            input.next_line().await?
        } else {
            match tokio::time::timeout(idle_write_tx_limit, input.next_line()).await {
                Ok(line) => line?,
                Err(_deadline) => {
                    connection.execute("ROLLBACK", ()).await?;
                    tracing::warn!(
                        idle_ms = idle_write_tx_limit.as_millis() as u64,
                        "rolled back an idle write transaction; client stalled"
                    );
                    continue;
                }
            }
        };
        let Some(line) = line else { break };
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{duplex, BufReader};

    async fn seed(path: &std::path::Path) {
        let database = turso::Builder::new_local(path.to_str().unwrap())
            .experimental_multiprocess_wal(true)
            .build()
            .await
            .unwrap();
        let connection = database.connect().unwrap();
        connection.busy_timeout(Duration::from_secs(30)).unwrap();
        connection
            .execute_batch("PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;")
            .await
            .unwrap();
        connection
            .execute("CREATE TABLE probe(id INTEGER)", ())
            .await
            .unwrap();
    }

    type ResponseLines = tokio::io::Lines<BufReader<tokio::io::DuplexStream>>;

    async fn next_response(responses: &mut ResponseLines) -> Value {
        let line = responses.next_line().await.unwrap().expect("response line");
        serde_json::from_str(&line).unwrap()
    }

    #[tokio::test]
    async fn stalled_client_has_its_open_write_transaction_rolled_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge.db");
        seed(&path).await;

        let (mut request_tx, request_rx) = duplex(4096);
        let (response_rx, response_tx) = duplex(4096);
        let mut responses = BufReader::new(response_rx).lines();
        let task = tokio::spawn(serve_streams(
            path,
            false,
            Duration::from_millis(150),
            BufReader::new(request_rx),
            response_tx,
        ));

        request_tx
            .write_all(b"{\"sql\":\"BEGIN IMMEDIATE\",\"parameters\":[]}\n")
            .await
            .unwrap();
        let begun = next_response(&mut responses).await;
        assert_eq!(begun["ok"], json!(true));
        assert_eq!(begun["autocommit"], json!(false));

        request_tx
            .write_all(b"{\"sql\":\"INSERT INTO probe VALUES (7)\",\"parameters\":[]}\n")
            .await
            .unwrap();
        let inserted = next_response(&mut responses).await;
        assert_eq!(inserted["changes"], json!(1));

        // Stall far past the watchdog deadline while the transaction is open.
        tokio::time::sleep(Duration::from_millis(500)).await;

        request_tx
            .write_all(b"{\"sql\":\"SELECT count(*) FROM probe\",\"parameters\":[]}\n")
            .await
            .unwrap();
        let after = next_response(&mut responses).await;
        assert_eq!(
            after["autocommit"],
            json!(true),
            "the watchdog must have rolled the idle transaction back"
        );
        assert_eq!(
            after["rows"][0][0],
            json!(0),
            "the rolled-back insert must not be visible"
        );

        drop(request_tx);
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn active_client_keeps_its_transaction_across_short_pauses() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge.db");
        seed(&path).await;

        let (mut request_tx, request_rx) = duplex(4096);
        let (response_rx, response_tx) = duplex(4096);
        let mut responses = BufReader::new(response_rx).lines();
        let task = tokio::spawn(serve_streams(
            path,
            false,
            Duration::from_millis(500),
            BufReader::new(request_rx),
            response_tx,
        ));

        for sql in ["BEGIN IMMEDIATE", "INSERT INTO probe VALUES (1)"] {
            request_tx
                .write_all(format!("{{\"sql\":\"{sql}\",\"parameters\":[]}}\n").as_bytes())
                .await
                .unwrap();
            let response = next_response(&mut responses).await;
            assert_eq!(response["ok"], json!(true));
        }

        // A pause well under the deadline must NOT trigger the watchdog.
        tokio::time::sleep(Duration::from_millis(50)).await;
        request_tx
            .write_all(b"{\"sql\":\"COMMIT\",\"parameters\":[]}\n")
            .await
            .unwrap();
        let committed = next_response(&mut responses).await;
        assert_eq!(committed["ok"], json!(true));

        request_tx
            .write_all(b"{\"sql\":\"SELECT count(*) FROM probe\",\"parameters\":[]}\n")
            .await
            .unwrap();
        let after = next_response(&mut responses).await;
        assert_eq!(after["rows"][0][0], json!(1), "the commit must survive");

        drop(request_tx);
        task.await.unwrap().unwrap();
    }
}
