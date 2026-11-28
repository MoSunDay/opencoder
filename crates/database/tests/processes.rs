//! Multi-process contract for one file-backed database: every process opens
//! it as a writer and transactions serialize through the engine's
//! multi-process WAL; a reader in a third process sees exactly the committed
//! prefix of every writer; the passive checkpoint runs beside live writers;
//! offline maintenance (the TRUNCATE checkpoint) fails fast until every
//! writer is gone and then produces a self-contained file. The schema
//! exercises the engine features a plain `id, value` table hides:
//! AUTOINCREMENT sequencing, a secondary index and overflow pages (payloads
//! larger than one page).
use opencoder_database::checkpoint::{self, Outcome};
use opencoder_database::{busy, maintenance, Database};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Writer processes beside the parent; the parent is writer `WRITERS`.
const WRITERS: i64 = 4;
/// Reader processes observing the writers.
const READERS: usize = 2;
const ROWS_PER_WRITER: i64 = 25;
const TOTAL: i64 = (WRITERS + 1) * ROWS_PER_WRITER;
/// `payload` exceeds the page size, so every row spills into overflow pages.
const PAYLOAD_BYTES: usize = 12 * 1024;
const SCHEMA: &str = "CREATE TABLE items(
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    label TEXT NOT NULL,
    value INTEGER NOT NULL,
    payload BLOB NOT NULL);
CREATE INDEX idx_items_label ON items(label);";

/// Child contract: the database (set for every child), the writer index (set
/// for writers only) and the file the parent writes to let writers finish.
const CHILD_DATABASE: &str = "OPENCODER_TEST_DATABASE";
const CHILD_WRITER: &str = "OPENCODER_TEST_WRITER";
const CHILD_RELEASE: &str = "OPENCODER_TEST_RELEASE";

fn payload(value: i64) -> Vec<u8> {
    let byte = (value % 251) as u8 + 1;
    vec![byte; PAYLOAD_BYTES]
}

/// The value of row `row` written by `writer`.
fn value_of(writer: i64, row: i64) -> i64 {
    writer * 100 + row
}

fn label_of(writer: i64, row: i64) -> String {
    format!("label-{writer}-{row:03}")
}

/// Sum of the first `rows` values of `writer`.
fn prefix_sum(writer: i64, rows: i64) -> i64 {
    rows * value_of(writer, 0) + rows * (rows - 1) / 2
}

fn expected_total() -> i64 {
    (0..=WRITERS)
        .map(|writer| prefix_sum(writer, ROWS_PER_WRITER))
        .sum()
}

/// One writer transaction: the begin is retried while another process holds
/// the engine's WAL writer lock, exactly as every application write does.
async fn insert(database: &Database, writer: i64, row: i64) {
    let value = value_of(writer, row);
    let label = label_of(writer, row);
    let bytes = payload(value);
    database
        .write(move |connection| async move {
            let conn = connection.clone();
            busy::run_tx(&connection, "BEGIN IMMEDIATE", || async move {
                conn.execute(
                    "INSERT INTO items(label, value, payload) VALUES(?1, ?2, ?3)",
                    turso::params![label, value, bytes],
                )
                .await?;
                Ok(())
            })
            .await
        })
        .await
        .unwrap();
}

/// Rows per writer seen in one snapshot: `(writer, rows, sum)`. A snapshot
/// of a WAL holds a prefix of every writer's sequence, so a consistent view
/// satisfies `sum == prefix_sum(writer, rows)` for every writer.
async fn per_writer(database: &Database) -> Vec<(i64, i64, i64)> {
    database
        .read(|connection| async move {
            let mut rows = connection
                .query(
                    "SELECT value / 100, count(*), sum(value) FROM items GROUP BY value / 100 \
                     ORDER BY value / 100",
                    (),
                )
                .await?;
            let mut seen = Vec::new();
            while let Some(row) = rows.next().await? {
                seen.push((
                    row.get(0)?,
                    row.get(1)?,
                    row.get::<Option<i64>>(2)?.unwrap_or(0),
                ));
            }
            Ok(seen)
        })
        .await
        .unwrap()
}

/// Whether `seen` is a consistent prefix view; returns the row count.
fn check_prefixes(seen: &[(i64, i64, i64)]) -> i64 {
    let mut total = 0;
    for &(writer, rows, sum) in seen {
        assert!((0..=WRITERS).contains(&writer), "unknown writer {writer}");
        assert!(
            rows <= ROWS_PER_WRITER,
            "writer {writer} wrote more than it owns"
        );
        assert_eq!(
            sum,
            prefix_sum(writer, rows),
            "writer {writer}: a torn prefix"
        );
        total += rows;
    }
    total
}

async fn seed(path: &Path) -> Database {
    let database = Database::open(path).await.unwrap();
    database
        .write(|connection| async move {
            connection.execute_batch(SCHEMA).await?;
            Ok(())
        })
        .await
        .unwrap();
    database
}

async fn wait_for_release(release: &Path) {
    let deadline = Instant::now() + Duration::from_secs(120);
    while !release.exists() {
        assert!(
            Instant::now() < deadline,
            "the parent never released the writers"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Child role: a writer commits its rows beside every other writer and keeps
/// its pool open until the parent releases it; a reader polls until it has
/// seen every committed row, checking each snapshot on the way.
async fn child(path: &Path, writer: Option<i64>, release: &Path) {
    match writer {
        Some(writer) => {
            let database = Database::open(path).await.unwrap();
            for row in 0..ROWS_PER_WRITER {
                insert(&database, writer, row).await;
            }
            wait_for_release(release).await;
            drop(database);
        }
        None => {
            let database = Database::open_reader(path).await.unwrap();
            let deadline = Instant::now() + Duration::from_secs(120);
            let mut previous: Vec<(i64, i64, i64)> = Vec::new();
            let mut complete = false;
            while !complete {
                assert!(
                    Instant::now() < deadline,
                    "the reader never saw every committed row: {previous:?}"
                );
                let seen = per_writer(&database).await;
                check_prefixes(&seen);
                for &(writer, rows, _) in &previous {
                    let now = seen
                        .iter()
                        .find(|(w, _, _)| *w == writer)
                        .map(|(_, rows, _)| *rows)
                        .unwrap_or(0);
                    assert!(now >= rows, "writer {writer}'s rows went backwards");
                }
                complete = seen.iter().map(|(_, rows, _)| rows).sum::<i64>() == TOTAL;
                previous = seen;
                if !complete {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            }
            assert!(
                database
                    .write(|connection| async move {
                        connection.execute("DELETE FROM items", ()).await?;
                        Ok(())
                    })
                    .await
                    .is_err(),
                "a reader must reject writes"
            );
        }
    }
}

fn spawn(test: &str, path: &Path, release: &Path, writer: Option<i64>) -> std::process::Child {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", test, "--nocapture"])
        .env(CHILD_DATABASE, path)
        .env(CHILD_RELEASE, release)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(writer) = writer {
        command.env(CHILD_WRITER, writer.to_string());
    } else {
        command.env_remove(CHILD_WRITER);
    }
    command.spawn().unwrap()
}

#[tokio::test]
async fn writer_processes_share_the_wal_and_maintenance_waits_for_them() {
    const TEST: &str = "writer_processes_share_the_wal_and_maintenance_waits_for_them";
    if let Ok(path) = std::env::var(CHILD_DATABASE) {
        let writer = std::env::var(CHILD_WRITER)
            .ok()
            .map(|writer| writer.parse().unwrap());
        child(
            Path::new(&path),
            writer,
            &PathBuf::from(std::env::var(CHILD_RELEASE).unwrap()),
        )
        .await;
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("shared.db");
    let release = directory.path().join("release");
    let database = seed(&path).await;
    let mut children = Vec::new();
    for writer in 0..WRITERS {
        children.push(spawn(TEST, &path, &release, Some(writer)));
    }
    for _ in 0..READERS {
        children.push(spawn(TEST, &path, &release, None));
    }
    // The parent writes beside the children through the same WAL.
    for row in 0..ROWS_PER_WRITER {
        insert(&database, WRITERS, row).await;
    }

    // Offline maintenance fails fast while the children write -- without a
    // pool in this process, the writers are the children alone.
    drop(database);
    let refused = maintenance::checkpoint(path.clone()).await.unwrap_err();
    assert!(
        refused.to_string().contains("writers still active"),
        "TRUNCATE must refuse beside the writer processes: {refused}"
    );
    // The passive pass is never refused beside them: no pool is alive here,
    // so the pass opens one private engine under its own shared lease.
    let opens = checkpoint::standalone_opens(&path);
    let outcome = checkpoint::run_once(&path, 0).await.unwrap();
    assert!(
        matches!(outcome, Outcome::Merged { .. } | Outcome::Busy),
        "the pass runs or skips on a busy WAL, never refuses: {outcome:?}"
    );
    assert_eq!(
        checkpoint::standalone_opens(&path),
        opens + 1,
        "the standalone pass opened exactly one private engine"
    );
    // An online copy beside the writers is a consistent prefix view too.
    let copy = directory.path().join("snapshot.db");
    maintenance::backup(path.clone(), copy.clone())
        .await
        .unwrap();
    let snapshot = maintenance::query(
        copy,
        "SELECT value / 100, count(*), sum(value) FROM items GROUP BY value / 100".into(),
        vec![],
    )
    .await
    .unwrap();
    let seen: Vec<(i64, i64, i64)> = snapshot["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row[0].as_i64().unwrap(),
                row[1].as_i64().unwrap(),
                row[2].as_i64().unwrap(),
            )
        })
        .collect();
    let copied = check_prefixes(&seen);
    assert!(copied > 0, "the copy was taken beside live writers");

    // Let the writers finish and collect their verdicts.
    std::fs::write(&release, b"").unwrap();
    for child in children {
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "child failed: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let result = maintenance::query(
        path.clone(),
        "SELECT count(*), sum(value), max(id) FROM items".into(),
        vec![],
    )
    .await
    .unwrap();
    assert_eq!(
        result["rows"],
        json!([[TOTAL, expected_total(), TOTAL]]),
        "every row of every writer committed once; AUTOINCREMENT ids track the rows"
    );
    let report = maintenance::query(path.clone(), "PRAGMA integrity_check".into(), vec![])
        .await
        .unwrap();
    assert_eq!(report["rows"], json!([["ok"]]));

    // Every writer gone: the stopped checkpoint produces a self-contained file.
    maintenance::checkpoint(path.clone()).await.unwrap();
    assert_eq!(
        checkpoint::wal_size(&path),
        0,
        "TRUNCATE must leave the WAL empty"
    );
    let reopened = Database::open(&path).await.unwrap();
    assert_eq!(per_writer(&reopened).await.len(), WRITERS as usize + 1);
    drop(reopened);
}
