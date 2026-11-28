//! One native connection for maintenance clients that need a transaction.
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt};

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

/// Read newline-delimited requests from stdin; unfinished transactions roll
/// back. The bridge is read-only unless the caller explicitly opts into
/// writes; a writable bridge is offline maintenance: it holds the database's
/// exclusive maintenance lease ([`crate::maintenance_lock`]) for its whole
/// life and fails fast with [`crate::maintenance_lock::WritersActive`] while
/// any writer is alive.
pub async fn serve(path: PathBuf, read_only: bool) -> Result<()> {
    let input = tokio::io::BufReader::new(tokio::io::stdin());
    let output = tokio::io::stdout();
    crate::runtime::run(
        true,
        serve_streams(path, read_only, IDLE_WRITE_TX_LIMIT, input, output),
    )
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
    ensure!(
        path.is_file(),
        "database file does not exist: {}",
        path.display()
    );
    // One canonical path for the maintenance lease and the engine: a bridge
    // opened through a symlink must lease and write the file the symlink
    // names, not grow a second WAL companion next to the link.
    let canonical = path.canonicalize()?;
    let path = canonical.as_path();
    // The engine takes a string path: refuse one it cannot open before the
    // lease is taken, so a refused bridge leaves no lease file behind.
    let name = crate::engine::engine_name(path)?;
    // A writable bridge is offline maintenance: it takes the exclusive
    // maintenance lease before the engine opens the file, never waits for a
    // live writer and releases the lease only when the bridge ends. Read-only
    // bridges take no lease, so inspection keeps working under live writers.
    let _lease = if read_only {
        None
    } else {
        Some(crate::maintenance_lock::try_acquire_exclusive(path)?)
    };
    // The engine is opened while the lease is held and declared after it, so
    // it is dropped before the lease releases.
    let database = crate::engine::open(name, read_only, path).await?;
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

    /// Open the bridge over private streams: `(request sink, task, responses)`.
    fn bridge(
        path: std::path::PathBuf,
        read_only: bool,
    ) -> (
        tokio::io::DuplexStream,
        tokio::task::JoinHandle<Result<()>>,
        ResponseLines,
    ) {
        let (request_tx, request_rx) = duplex(4096);
        let (response_rx, response_tx) = duplex(4096);
        let responses = BufReader::new(response_rx).lines();
        let task = tokio::spawn(serve_streams(
            path,
            read_only,
            Duration::from_millis(500),
            BufReader::new(request_rx),
            response_tx,
        ));
        (request_tx, task, responses)
    }

    async fn send(request_tx: &mut tokio::io::DuplexStream, sql: &str) {
        request_tx
            .write_all(format!("{{\"sql\":\"{sql}\",\"parameters\":[]}}\n").as_bytes())
            .await
            .unwrap();
    }

    /// A writable bridge is offline maintenance on the database's exclusive
    /// maintenance lease: while a writer pool is alive (here or in another
    /// process) the bridge fails fast with `WritersActive`, and while the
    /// bridge is alive a writer's shared lease is refused with
    /// `MaintenanceActive`. A read-only bridge takes no lease and keeps
    /// serving under the pool. Once each side is gone the other may proceed.
    #[tokio::test]
    async fn a_writable_bridge_and_writer_pools_exclude_each_other() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge.db");
        let pool = crate::Database::open(&path).await.unwrap();
        pool.write(|connection| async move {
            connection
                .execute("CREATE TABLE probe(id INTEGER)", ())
                .await?;
            Ok(())
        })
        .await
        .unwrap();

        let (mut request_tx, task, mut responses) = bridge(path.clone(), true);
        send(&mut request_tx, "SELECT count(*) FROM probe").await;
        let served = next_response(&mut responses).await;
        assert_eq!(served["rows"][0][0], json!(0));
        drop(request_tx);
        task.await.unwrap().unwrap();

        let (_request_tx, task, _responses) = bridge(path.clone(), false);
        let refused = task
            .await
            .unwrap()
            .expect_err("the bridge must not start beside a live writer");
        assert!(
            crate::maintenance_lock::is_writers_active(&refused),
            "the writable bridge must meet the pool's shared lease: {refused:#}"
        );
        let conflict = refused
            .chain()
            .find_map(|cause| cause.downcast_ref::<crate::maintenance_lock::WritersActive>())
            .expect("the conflict is carried in the chain");
        assert_eq!(conflict.database, path.canonicalize().unwrap());

        drop(pool);
        let (mut request_tx, task, mut responses) = bridge(path.clone(), false);
        send(&mut request_tx, "INSERT INTO probe VALUES (1)").await;
        assert_eq!(next_response(&mut responses).await["ok"], json!(true));

        // The bridge holds the exclusive lease: a writer's shared lease is
        // refused at once with `MaintenanceActive` (a pool open would wait the
        // lease out), a reader open is not refused.
        let refused = crate::maintenance_lock::acquire_shared(&path.canonicalize().unwrap())
            .expect_err("a writer must not lease beside a writable bridge");
        assert!(
            crate::maintenance_lock::is_maintenance_active(&refused),
            "{refused:#}"
        );
        drop(crate::Database::open_reader(&path).await.unwrap());

        drop(request_tx);
        task.await.unwrap().unwrap();
        let pool = crate::Database::open(&path)
            .await
            .expect("the bridge must release its lease when it ends");
        let count = pool
            .read(|connection| async move {
                let mut rows = connection.query("SELECT count(*) FROM probe", ()).await?;
                Ok(rows.next().await?.unwrap().get::<i64>(0)?)
            })
            .await
            .unwrap();
        assert_eq!(count, 1, "the bridge's insert must be visible");
    }

    /// A bridge opened through a symlink leases and writes the file the link
    /// names: the lease is recorded for the resolved path and no lease file
    /// appears next to the link.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_writable_bridge_through_a_symlink_leases_the_file_it_names() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.db");
        let pool = crate::Database::open(&target).await.unwrap();
        pool.write(|connection| async move {
            connection
                .execute("CREATE TABLE probe(id INTEGER)", ())
                .await?;
            Ok(())
        })
        .await
        .unwrap();
        drop(pool);
        let link = dir.path().join("link.db");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let (mut request_tx, task, mut responses) = bridge(link.clone(), false);
        send(&mut request_tx, "INSERT INTO probe VALUES (1)").await;
        assert_eq!(next_response(&mut responses).await["ok"], json!(true));
        let target = target.canonicalize().unwrap();
        let refused = crate::maintenance_lock::try_acquire_exclusive(&target)
            .expect_err("the bridge must hold the lease for the resolved file");
        assert!(
            crate::maintenance_lock::is_writers_active(&refused),
            "{refused}"
        );
        assert!(
            !crate::maintenance_lock::lock_path(&link).exists(),
            "no lease file may appear next to the link"
        );
        drop(request_tx);
        task.await.unwrap().unwrap();
        let lease = crate::maintenance_lock::acquire_shared(&target)
            .expect("the bridge must release its lease when it ends");
        assert_eq!(
            lease.lock_path(),
            crate::maintenance_lock::lock_path(&target)
        );
        assert_eq!(lease.database(), target.as_path());
        assert_eq!(lease.mode(), crate::maintenance_lock::Mode::Shared);
    }

    /// The engine takes a string path, so a path that is not UTF-8 is refused
    /// by both bridge modes before the lease is taken and before the engine
    /// opens the file: no lease file and no companion appear beside it. A
    /// file system that refuses such a name cannot express the case.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_non_utf8_path_is_refused_before_the_bridge_takes_the_lease() {
        use std::os::unix::ffi::OsStringExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir
            .path()
            .join(std::ffi::OsString::from_vec(b"invalid-\xff.db".to_vec()));
        if let Err(error) = std::fs::write(&path, b"") {
            eprintln!("skipping: the file system refuses a non-UTF-8 name: {error}");
            return;
        }
        for read_only in [false, true] {
            let (_request_tx, task, _responses) = bridge(path.clone(), read_only);
            let refused = task
                .await
                .unwrap()
                .expect_err("a non-UTF-8 path must be refused");
            assert!(
                refused.to_string().contains("database path must be UTF-8"),
                "the refusal must name the cause: {refused:#}"
            );
        }
        let mut left = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        left.sort();
        assert_eq!(
            left,
            vec![path.file_name().unwrap().to_os_string()],
            "no lease file or companion may appear beside the database"
        );
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
