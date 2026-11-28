//! Soak for the multi-writer WAL: one writer process is killed with SIGKILL
//! while it commits beside others. The survivors must keep committing
//! afterwards without an error, the file must pass `PRAGMA integrity_check`,
//! every writer's committed rows must be an exact prefix of its sequence
//! with no duplicate (the unique index would reject one), and the killed
//! writer's maintenance lease must die with the process, so offline
//! maintenance is admitted once the survivors exit. Ignored by default:
//! `cargo test -p opencoder-database --test multi_writer_soak -- --ignored`.
use opencoder_database::{busy, checkpoint, maintenance, maintenance_lock, Database};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Writer processes; `VICTIM` is the one the parent kills.
const WRITERS: i64 = 4;
const VICTIM: i64 = 0;
/// The victim is killed once it has committed this many rows; survivors
/// must commit this many more rows after the kill before they are released.
const MIN_ROWS: i64 = 10;
const PROGRESS_AFTER_KILL: i64 = 5;
/// Child contract: the database, the writer index and the parent's release file.
const CHILD_DATABASE: &str = "OPENCODER_TEST_SOAK_DATABASE";
const CHILD_WRITER: &str = "OPENCODER_TEST_SOAK_WRITER";
const CHILD_RELEASE: &str = "OPENCODER_TEST_SOAK_RELEASE";
const DEADLINE: Duration = Duration::from_secs(120);

fn value_of(writer: i64, row: i64) -> i64 {
    writer * 1_000_000 + row
}

fn prefix_sum(writer: i64, rows: i64) -> i64 {
    (0..rows).map(|row| value_of(writer, row)).sum()
}

/// One committed transaction per row, serialized with every other writer
/// through the engine's WAL writer lock.
async fn insert(database: &Database, writer: i64, row: i64) {
    let value = value_of(writer, row);
    database
        .write(move |connection| async move {
            let conn = connection.clone();
            busy::run_tx(&connection, "BEGIN IMMEDIATE", || async move {
                conn.execute(
                    "INSERT INTO items(writer, row, value) VALUES(?1, ?2, ?3)",
                    turso::params![writer, row, value],
                )
                .await?;
                Ok(())
            })
            .await
        })
        .await
        .unwrap();
}

/// `writer -> rows` in one snapshot.
async fn rows_per_writer(database: &Database) -> Vec<i64> {
    database
        .read(|connection| async move {
            let mut rows = connection
                .query("SELECT writer, count(*) FROM items GROUP BY writer", ())
                .await?;
            let mut counts = vec![0; WRITERS as usize];
            while let Some(row) = rows.next().await? {
                let writer: i64 = row.get(0)?;
                counts[writer as usize] = row.get(1)?;
            }
            Ok(counts)
        })
        .await
        .unwrap()
}

/// Child role: commit rows back to back until the parent releases it.
async fn child(path: &Path, release: &Path, writer: i64) {
    let database = Database::open(path).await.unwrap();
    for row in 0i64.. {
        if release.exists() {
            break;
        }
        insert(&database, writer, row).await;
    }
}

fn spawn(test: &str, path: &Path, release: &Path, writer: i64) -> Child {
    Command::new(std::env::current_exe().unwrap())
        // The child must run the ignored test it is re-executed as.
        .args(["--exact", test, "--nocapture", "--ignored"])
        .env(CHILD_DATABASE, path)
        .env(CHILD_RELEASE, release)
        .env(CHILD_WRITER, writer.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

/// Output of an exited child: the pipes read to their end.
fn output_of(output: std::process::Output) -> String {
    format!(
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// Fail if any writer exited: they all run until released.
fn assert_alive(children: &mut [&mut Child]) {
    for child in children {
        if let Some(status) = child.try_wait().unwrap() {
            panic!("a writer exited before the release ({status})");
        }
    }
}

/// Wait until `progress(counts)` holds for the committed counts.
async fn wait_for_counts(
    reader: &Database,
    children: &mut [&mut Child],
    progress: impl Fn(&[i64]) -> bool,
) -> Vec<i64> {
    let deadline = Instant::now() + DEADLINE;
    loop {
        let counts = rows_per_writer(reader).await;
        if progress(&counts) {
            return counts;
        }
        assert!(Instant::now() < deadline, "writers stalled at {counts:?}");
        assert_alive(children);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test]
#[ignore = "soak: run explicitly with --ignored"]
async fn a_writer_killed_with_sigkill_does_not_stop_the_others() {
    const TEST: &str = "a_writer_killed_with_sigkill_does_not_stop_the_others";
    if let Ok(path) = std::env::var(CHILD_DATABASE) {
        let writer: i64 = std::env::var(CHILD_WRITER).unwrap().parse().unwrap();
        child(
            Path::new(&path),
            &PathBuf::from(std::env::var(CHILD_RELEASE).unwrap()),
            writer,
        )
        .await;
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("soak.db");
    let release = directory.path().join("release");
    let database = Database::open(&path).await.unwrap();
    database
        .write(|connection| async move {
            connection
                .execute_batch(
                    "CREATE TABLE items(
                        id INTEGER PRIMARY KEY AUTOINCREMENT,
                        writer INTEGER NOT NULL,
                        row INTEGER NOT NULL,
                        value INTEGER NOT NULL);
                    CREATE UNIQUE INDEX idx_items_writer_row ON items(writer, row);",
                )
                .await?;
            Ok(())
        })
        .await
        .unwrap();
    let mut children: Vec<Child> = (0..WRITERS)
        .map(|writer| spawn(TEST, &path, &release, writer))
        .collect();
    let reader = Database::open_reader(&path).await.unwrap();

    // Every writer commits, then the victim dies mid-write.
    let at_kill = {
        let mut writers: Vec<&mut Child> = children.iter_mut().collect();
        wait_for_counts(&reader, &mut writers, |counts| {
            counts.iter().all(|&rows| rows >= MIN_ROWS)
        })
        .await
    };
    let mut victim = children.remove(VICTIM as usize);
    victim.kill().expect("SIGKILL the victim");
    let killed = victim.wait_with_output().unwrap();
    assert!(
        !killed.status.success(),
        "the victim must die by the signal: {}",
        output_of(killed)
    );

    // The survivors keep committing after the kill, then finish on release.
    let final_counts = {
        let mut survivors: Vec<&mut Child> = children.iter_mut().collect();
        wait_for_counts(&reader, &mut survivors, |counts| {
            counts.iter().enumerate().all(|(writer, &rows)| {
                writer as i64 == VICTIM || rows >= at_kill[writer] + PROGRESS_AFTER_KILL
            })
        })
        .await
    };
    std::fs::write(&release, b"").unwrap();
    for child in children {
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "a survivor failed after the kill: {}",
            output_of(output)
        );
    }
    // The parent's own pool is not part of the lease check below.
    drop(database);

    // The killed process released its lease with its file descriptors: with
    // every writer gone, offline maintenance is admitted and truncates.
    let key = path.canonicalize().unwrap();
    drop(maintenance_lock::try_acquire_exclusive(&key).unwrap());
    maintenance::checkpoint(path.clone()).await.unwrap();
    assert_eq!(
        checkpoint::wal_size(&path),
        0,
        "TRUNCATE leaves the WAL empty"
    );

    // Every writer's rows form an exact prefix of its sequence: no torn
    // transaction, no duplicate after the crash of the victim.
    let seen = maintenance::query(
        path.clone(),
        "SELECT writer, count(*), sum(value) FROM items GROUP BY writer ORDER BY writer".into(),
        vec![],
    )
    .await
    .unwrap();
    let rows = seen["rows"].as_array().unwrap();
    assert_eq!(
        rows.len(),
        WRITERS as usize,
        "every writer committed rows: {seen}"
    );
    for (writer, row) in rows.iter().enumerate() {
        let writer = writer as i64;
        let count = row[1].as_i64().unwrap();
        assert_eq!(
            row[2].as_i64().unwrap(),
            prefix_sum(writer, count),
            "writer {writer}: a torn prefix"
        );
        if writer == VICTIM {
            assert!(count >= MIN_ROWS, "the victim's committed rows: {seen}");
        } else {
            assert!(
                count >= final_counts[writer as usize],
                "writer {writer}: {seen}"
            );
        }
    }
    let report = maintenance::query(path.clone(), "PRAGMA integrity_check".into(), vec![])
        .await
        .unwrap();
    assert_eq!(report["rows"], json!([["ok"]]));
}
