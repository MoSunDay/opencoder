//! Maintenance leases beside the pool: every writer pool holds a shared
//! lease for its lifetime, offline maintenance takes an exclusive one and
//! fails fast with `WritersActive` while any writer -- here or in another
//! process -- is alive, a reader takes nothing, and a dropped owner releases
//! its lease at once. Writers never exclude each other: a second writer, in
//! this or another process, opens and commits beside the first.
use opencoder_database::maintenance_lock::{
    self, is_writers_active, lock_path, Mode, WritersActive,
};
use opencoder_database::{maintenance, Database};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

/// Env contract with the child process: the database it opens as a writer.
const CHILD_DB: &str = "OPENCODER_TEST_LEASE_DB";

fn key(path: &Path) -> PathBuf {
    path.canonicalize().unwrap()
}

/// Take the exclusive lease of the canonical `db` and require the refusal to
/// be a live writer naming that database and its lease file.
fn refused_exclusive(db: &Path) -> anyhow::Error {
    let error = maintenance_lock::try_acquire_exclusive(db)
        .expect_err("the exclusive lease must be refused beside a live writer");
    assert!(is_writers_active(&error), "{error}");
    let conflict = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<WritersActive>())
        .expect("the conflict names the database and its lease file");
    assert_eq!(conflict.database, db);
    assert_eq!(conflict.lock, lock_path(db));
    assert!(
        error.to_string().contains("active writers"),
        "the refusal must say what is alive: {error}"
    );
    error
}

async fn seeded(path: &Path) -> Database {
    let database = Database::open(path).await.unwrap();
    database
        .write(|connection| async move {
            connection
                .execute_batch(
                    "CREATE TABLE items(id INTEGER PRIMARY KEY, value INTEGER); \
                     INSERT INTO items VALUES(1, 10);",
                )
                .await?;
            Ok(())
        })
        .await
        .unwrap();
    database
}

/// Child body: open the database as a writer (the pool leases it), report
/// it and keep the pool until the parent closes the pipe.
async fn child_writer(db: &Path) {
    let pool = seeded(db).await;
    println!("held");
    std::io::stdout().flush().unwrap();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).unwrap();
    drop(pool);
}

/// A child running `test` in its writer role, waited until it holds the pool.
struct WriterChild {
    child: Child,
    stdin: ChildStdin,
}

impl WriterChild {
    fn spawn(test: &str, db: &Path) -> (Self, BufReader<ChildStdout>) {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test, "--nocapture"])
            .env(CHILD_DB, db)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        // The child is this test binary under the harness, which prints its
        // own banner first: wait for the writer's line, not the first line.
        let mut seen = String::new();
        loop {
            let mut line = String::new();
            if stdout.read_line(&mut line).unwrap() == 0 {
                panic!("the child exited before holding the pool: {seen}");
            }
            seen.push_str(&line);
            if line.trim() == "held" {
                break;
            }
        }
        (Self { child, stdin }, stdout)
    }

    /// Close the pipe: the child drops its pool and exits.
    fn release(self) {
        let Self { mut child, stdin } = self;
        drop(stdin);
        let status = child.wait().unwrap();
        assert!(status.success(), "the child released the pool cleanly");
    }
}

/// The lease file is created next to the database on first use, private to
/// the user, names the database it was taken for and goes away with its owner.
#[test]
fn a_shared_lease_is_recorded_next_to_the_database_and_released_on_drop() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("lease.db");
    let lease = maintenance_lock::acquire_shared(&database).unwrap();
    assert_eq!(lease.mode(), Mode::Shared);
    assert_eq!(lease.database(), database);
    assert_eq!(lease.lock_path(), lock_path(&database));
    assert_eq!(
        lease.lock_path(),
        directory.path().join("lease.db.maintenance.lock")
    );
    assert!(lease.lock_path().is_file());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(lease.lock_path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600,
            "the lease file is private to the user"
        );
    }
    drop(lease);
    let exclusive = maintenance_lock::try_acquire_exclusive(&database).unwrap();
    assert_eq!(exclusive.mode(), Mode::Exclusive);
    assert_eq!(exclusive.database(), database);
}

/// Any number of writers coexist; none of them coexists with offline
/// maintenance, in either direction.
#[test]
fn shared_leases_coexist_and_exclude_an_exclusive_one() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("shared.db");
    let first = maintenance_lock::acquire_shared(&database).unwrap();
    let second = maintenance_lock::acquire_shared(&database).unwrap();
    refused_exclusive(&database);
    drop(first);
    refused_exclusive(&database);
    drop(second);

    let exclusive = maintenance_lock::try_acquire_exclusive(&database).unwrap();
    let refused = maintenance_lock::acquire_shared(&database).unwrap_err();
    assert!(is_writers_active(&refused), "{refused}");
    drop(exclusive);
    drop(maintenance_lock::acquire_shared(&database).unwrap());
}

/// Only a lease conflict counts as a live writer, and it is found anywhere in
/// the chain; any other failure keeps its own meaning.
#[test]
fn a_conflict_is_recognised_through_context_and_nothing_else_counts() {
    let conflict = anyhow::Error::new(WritersActive {
        database: PathBuf::from("/data/x.db"),
        lock: PathBuf::from("/data/x.db.maintenance.lock"),
    });
    assert!(is_writers_active(&conflict));
    assert!(is_writers_active(
        &conflict.context("lock database maintenance lease")
    ));
    assert!(!is_writers_active(&anyhow::anyhow!("plain failure")));
    assert!(!is_writers_active(
        &anyhow::anyhow!("plain failure").context("lock database maintenance lease")
    ));
    assert!(!is_writers_active(
        &anyhow::Error::new(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "locked"
        ))
        .context("lock database maintenance lease")
    ));
}

/// A writer pool leases its database for its whole lifetime -- a repeated
/// open shares that pool -- while a reader takes nothing: offline maintenance
/// is refused beside the writer and admitted beside the reader alone.
#[tokio::test]
async fn a_writer_pool_leases_its_database_and_a_reader_takes_nothing() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("pool.db");
    let writer = seeded(&path).await;
    let key = key(&path);
    refused_exclusive(&key);

    let again = Database::open(&path).await.unwrap();
    again
        .write(|connection| async move {
            connection
                .execute("INSERT INTO items VALUES(2, 20)", ())
                .await?;
            Ok(())
        })
        .await
        .unwrap();
    refused_exclusive(&key);

    let reader = Database::open_reader(&path).await.unwrap();
    drop(again);
    drop(writer);
    let lease = maintenance_lock::try_acquire_exclusive(&key)
        .expect("a reader holds no lease: maintenance is admitted beside it");
    drop(lease);
    let value = reader
        .read(|connection| async move {
            let mut rows = connection.query("SELECT sum(value) FROM items", ()).await?;
            Ok(rows.next().await?.unwrap().get::<i64>(0)?)
        })
        .await
        .unwrap();
    assert_eq!(value, 30);
}

/// The writer is another process: its pool blocks offline maintenance here
/// -- the exclusive lease and the TRUNCATE checkpoint both fail fast -- while
/// a second writer here opens, commits and is read back by a third open.
/// Once the other process is gone maintenance is admitted.
#[tokio::test]
async fn a_writer_in_another_process_blocks_maintenance_until_it_exits() {
    const TEST: &str = "a_writer_in_another_process_blocks_maintenance_until_it_exits";
    if let Ok(db) = std::env::var(CHILD_DB) {
        child_writer(Path::new(&db)).await;
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("other.db");
    let (child, _stdout) = WriterChild::spawn(TEST, &path);
    let key = key(&path);

    refused_exclusive(&key);
    let truncate = maintenance::checkpoint(path.clone()).await.unwrap_err();
    assert!(
        truncate.to_string().contains("writers still active"),
        "TRUNCATE must refuse while the other process writes: {truncate}"
    );

    let second = Database::open(&path).await.unwrap();
    second
        .write(|connection| async move {
            connection
                .execute("INSERT INTO items VALUES(2, 20)", ())
                .await?;
            Ok(())
        })
        .await
        .unwrap();
    let reader = Database::open_reader(&path).await.unwrap();
    let count = reader
        .read(|connection| async move {
            let mut rows = connection.query("SELECT count(*) FROM items", ()).await?;
            Ok(rows.next().await?.unwrap().get::<i64>(0)?)
        })
        .await
        .unwrap();
    assert_eq!(count, 2, "both processes' rows are visible");
    drop(second);
    drop(reader);
    refused_exclusive(&key);

    child.release();
    let lease = maintenance_lock::try_acquire_exclusive(&key)
        .expect("the exited writer released its lease");
    drop(lease);
    maintenance::checkpoint(path).await.unwrap();
}
