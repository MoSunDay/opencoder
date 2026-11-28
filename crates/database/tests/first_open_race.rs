//! Concurrent first opens of one path. A first open is the open that creates
//! the engine's WAL coordination companion (`<db>-tshm`): the engine creates
//! the file empty and writes its header afterwards, so another process
//! opening in between would see a short file. The pool runs a first open
//! under the exclusive maintenance lease and every other opener waits for
//! that lease, so all concurrent openers succeed and every writer's row is
//! committed exactly once.
//!
//! Two shapes are raced: a path that does not exist yet, and a database file
//! without its companions -- what an upgrade from an engine without the
//! coordination file, or a copy of the bare file, leaves behind. Every
//! process that touched the file before the race has exited: a live process
//! keeps its engine mapped, so the companions are never removed under one.
use opencoder_database::{busy, checkpoint, maintenance, Database};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const WRITERS: usize = 5;
/// Child contract: the database, the role, the writer index and the start file.
const CHILD_DATABASE: &str = "OPENCODER_TEST_RACE_DATABASE";
const CHILD_ROLE: &str = "OPENCODER_TEST_RACE_ROLE";
const CHILD_WRITER: &str = "OPENCODER_TEST_RACE_WRITER";
const CHILD_GO: &str = "OPENCODER_TEST_RACE_GO";

/// Writer role: wait for the start signal, then open the path as the first
/// writer would -- the schema bootstrap runs inside the same immediate
/// transaction as the first write.
async fn writer(path: &Path, go: &Path, writer: i64) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !go.exists() {
        assert!(
            Instant::now() < deadline,
            "the parent never started the children"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    let database = Database::open(path).await.unwrap();
    database
        .write(move |connection| {
            let work = connection.clone();
            async move {
                busy::run_tx(&connection, "BEGIN IMMEDIATE", move || async move {
                    work.execute_batch(
                        "CREATE TABLE IF NOT EXISTS items(id INTEGER PRIMARY KEY, writer INTEGER)",
                    )
                    .await?;
                    work.execute("INSERT INTO items(writer) VALUES(?1)", (writer,))
                        .await?;
                    Ok(())
                })
                .await
            }
        })
        .await
        .unwrap();
}

/// Schema role: create the file and leave it self-contained -- the schema is
/// checkpointed into the file and the WAL is empty.
async fn schema(path: &Path) {
    let database = Database::open(path).await.unwrap();
    database
        .write(|connection| async move {
            connection
                .execute_batch("CREATE TABLE items(id INTEGER PRIMARY KEY, writer INTEGER)")
                .await?;
            Ok(())
        })
        .await
        .unwrap();
    drop(database);
    maintenance::checkpoint(path.to_path_buf()).await.unwrap();
    assert_eq!(checkpoint::wal_size(path), 0);
}

fn spawn(test: &str, path: &Path, role: &str, go: Option<&Path>, writer: Option<i64>) -> Child {
    Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test, "--nocapture"])
        .env(CHILD_DATABASE, path)
        .env(CHILD_ROLE, role)
        .env(CHILD_GO, go.unwrap_or(path))
        .env(CHILD_WRITER, writer.unwrap_or_default().to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn run_child(test: &str, path: &Path, role: &str) {
    let output = spawn(test, path, role, None, None)
        .wait_with_output()
        .unwrap();
    assert!(
        output.status.success(),
        "the {role} child failed: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Start every writer at once and verify that each one committed once.
async fn race(test: &str, directory: &Path, path: &Path) {
    let go = directory.join("go");
    let children: Vec<Child> = (0..WRITERS)
        .map(|writer| spawn(test, path, "writer", Some(&go), Some(writer as i64)))
        .collect();
    tokio::time::sleep(Duration::from_millis(300)).await;
    std::fs::write(&go, b"").unwrap();
    for child in children {
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "a concurrent first open failed: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let database = Database::open(path).await.unwrap();
    let (count, distinct): (i64, i64) = database
        .read(|connection| async move {
            let mut rows = connection
                .query("SELECT count(*), count(DISTINCT writer) FROM items", ())
                .await?;
            let row = rows.next().await?.unwrap();
            Ok((row.get(0)?, row.get(1)?))
        })
        .await
        .unwrap();
    assert_eq!((count, distinct), (WRITERS as i64, WRITERS as i64));
}

async fn child_dispatch() {
    let Some(path) = std::env::var_os(CHILD_DATABASE) else {
        return;
    };
    let path = PathBuf::from(path);
    let go = PathBuf::from(std::env::var_os(CHILD_GO).unwrap());
    let role = std::env::var(CHILD_ROLE).unwrap();
    let index: i64 = std::env::var(CHILD_WRITER).unwrap().parse().unwrap();
    if role == "schema" {
        schema(&path).await;
    } else {
        writer(&path, &go, index).await;
    }
    // The parent's test body would run the race a second time.
    std::process::exit(0);
}

#[tokio::test]
async fn concurrent_first_opens_of_a_missing_file_all_succeed() {
    const TEST: &str = "concurrent_first_opens_of_a_missing_file_all_succeed";
    child_dispatch().await;
    let directory = tempfile::tempdir().unwrap();
    race(TEST, directory.path(), &directory.path().join("fresh.db")).await;
}

#[tokio::test]
async fn concurrent_first_opens_of_a_file_without_companions_all_succeed() {
    const TEST: &str = "concurrent_first_opens_of_a_file_without_companions_all_succeed";
    child_dispatch().await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("restored.db");
    run_child(TEST, &path, "schema");
    // The bare file is what the race needs: its checkpointed schema is in the
    // file and no process has it open, so the companions can go.
    for suffix in ["-wal", "-tshm"] {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        std::fs::remove_file(PathBuf::from(name)).unwrap();
    }
    race(TEST, directory.path(), &path).await;
}
