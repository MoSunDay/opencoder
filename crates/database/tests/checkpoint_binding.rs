//! Checkpoint eligibility is bound to what keeps a handle alive: a handle
//! whose pool is gone never checkpoints again, a re-opened pool publishes a
//! new handle the loop follows, the standalone entry opens a private engine
//! under its own shared maintenance lease (a writer, so a TRUNCATE never
//! rewrites the WAL under it) and never refuses because another writer is
//! alive, and no entry ever creates a database, a lease file or a marker.
use opencoder_database::checkpoint::{self, Outcome};
use opencoder_database::maintenance_lock::{self, is_writers_active};
use opencoder_database::{maintenance, Database};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A writer pool on `path` with one table and one row; reopening the same
/// file through this helper keeps the row stable.
async fn seeded(path: &Path) -> Database {
    let database = Database::open(path).await.unwrap();
    database
        .write(|connection| async move {
            connection
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS items(id INTEGER PRIMARY KEY, value INTEGER); \
                     INSERT OR REPLACE INTO items VALUES(1, 10);",
                )
                .await?;
            Ok(())
        })
        .await
        .unwrap();
    database
}

fn key(path: &Path) -> PathBuf {
    path.canonicalize().unwrap()
}

const POOL_GONE: &str = "bound to a pool that is gone";

/// The exclusive lease is refused because a writer is alive: `db` must be
/// the canonical path the lease was recorded under.
fn refused_exclusive(db: &Path) {
    let error = maintenance_lock::try_acquire_exclusive(db)
        .expect_err("offline maintenance must be refused beside a live writer");
    assert!(is_writers_active(&error), "{error}");
}

/// A handle retained past its pool's drop is dead for good: the loop prunes
/// it and every entry that takes the handle itself refuses it. The pool
/// released its lease with its drop, and what leases the path afterwards --
/// a standalone pass, a re-opened pool -- is a writer in its own right.
#[tokio::test]
async fn a_retained_handle_never_checkpoints_once_its_pool_is_gone() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("retained.db");
    let pool = seeded(&path).await;
    let handle = checkpoint::handle_for(&path).await.unwrap();
    let key = key(&path);
    assert!(handle.owner_alive());
    refused_exclusive(&key);

    drop(pool);
    assert!(
        !handle.owner_alive(),
        "the pool's drop must disarm the handle"
    );
    drop(
        maintenance_lock::try_acquire_exclusive(&key)
            .expect("the dropped pool must not keep its lease"),
    );
    assert_eq!(
        checkpoint::periodic_pass(&key, u64::MAX).await.unwrap(),
        None,
        "the loop prunes the orphaned handle and runs nothing"
    );
    for error in [
        checkpoint::checkpoint_passive(&handle).await.unwrap_err(),
        checkpoint::run_once_handle(&handle, 0).await.unwrap_err(),
        handle.prime().await.unwrap_err(),
    ] {
        assert!(
            error.to_string().contains(POOL_GONE),
            "the refusal must name the binding cause: {error}"
        );
    }

    // The standalone entry is a writer: a private engine under a shared lease
    // that a TRUNCATE checkpoint meets, released with the engine.
    let opens = checkpoint::standalone_opens(&path);
    let standalone = checkpoint::handle_for(&path).await.unwrap();
    assert!(
        !Arc::ptr_eq(&handle, &standalone) && standalone.owner_alive(),
        "handle_for must not hand out the dead handle"
    );
    assert_eq!(
        checkpoint::standalone_opens(&path),
        opens + 1,
        "the standalone pass opened one private engine"
    );
    refused_exclusive(&key);
    let truncate = maintenance::checkpoint(path.clone()).await.unwrap_err();
    assert!(
        truncate.to_string().contains("writers still active"),
        "a TRUNCATE must fail fast beside the standalone pass: {truncate}"
    );
    let outcome = checkpoint::run_once(&path, u64::MAX).await.unwrap();
    assert!(
        matches!(outcome, Outcome::Skipped { .. }),
        "under the gate nothing runs: {outcome:?}"
    );
    assert_eq!(
        checkpoint::standalone_opens(&path),
        opens + 2,
        "a standalone handle is never registered: each pass opens its own"
    );
    drop(standalone);
    drop(maintenance_lock::try_acquire_exclusive(&key).unwrap());

    // A writer opened afterwards is a writer again, never an owner of the
    // older handle, and the standalone pass runs beside it without refusing.
    let later = seeded(&path).await;
    assert!(!handle.owner_alive());
    let refused = checkpoint::run_once_handle(&handle, 0).await.unwrap_err();
    assert!(
        refused.to_string().contains(POOL_GONE),
        "a later writer must never qualify the older handle: {refused}"
    );
    refused_exclusive(&key);
    drop(later);
    drop(maintenance_lock::try_acquire_exclusive(&key).unwrap());
}

/// Re-opening the file publishes a new handle; the loop follows it and the
/// handle of the previous pool stays refused even though the path is written.
#[tokio::test]
async fn a_reopened_pool_publishes_a_new_handle_and_the_loop_follows_it() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("reopen.db");
    let first = seeded(&path).await;
    let old = checkpoint::handle_for(&path).await.unwrap();
    drop(first);

    let second = seeded(&path).await;
    let key = key(&path);
    let current = checkpoint::handle_for(&path).await.unwrap();
    assert!(
        !Arc::ptr_eq(&old, &current),
        "the re-opened pool must publish its own handle"
    );
    assert!(!old.owner_alive(), "the previous pool is still gone");
    assert!(
        checkpoint::periodic_pass(&key, u64::MAX)
            .await
            .unwrap()
            .is_some(),
        "the loop must run under the re-opened pool's handle"
    );
    let refused = checkpoint::run_once_handle(&old, 0).await.unwrap_err();
    assert!(
        refused.to_string().contains(POOL_GONE),
        "the superseded handle must stay refused: {refused}"
    );
    drop(second);
    assert!(
        !current.owner_alive(),
        "the new handle dies with its pool too"
    );
}

/// A checkpoint never creates a database: a missing file is refused before
/// the lease file, the marker or a WAL companion appears.
#[tokio::test]
async fn a_missing_database_is_refused_before_any_artifact_is_created() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("missing.db");
    for error in [
        checkpoint::run_once(&path, 0).await.unwrap_err(),
        checkpoint::handle_for(&path).await.err().unwrap(),
        maintenance::checkpoint(path.clone()).await.unwrap_err(),
    ] {
        assert!(
            error.to_string().contains("does not exist"),
            "the refusal must name the missing file: {error}"
        );
    }
    assert!(!path.exists(), "no database may be created");
    assert!(
        std::fs::read_dir(directory.path())
            .unwrap()
            .next()
            .is_none(),
        "the directory must stay empty: no lease file, marker or WAL"
    );
}

/// The same refusal through a dangling symlink: nothing may be created next
/// to the link, which names a file that does not exist.
#[cfg(unix)]
#[tokio::test]
async fn a_dangling_symlink_is_refused_without_materializing_its_target() {
    let directory = tempfile::tempdir().unwrap();
    let link = directory.path().join("link.db");
    std::os::unix::fs::symlink("absent/data.db", &link).unwrap();
    for error in [
        checkpoint::run_once(&link, 0).await.unwrap_err(),
        checkpoint::handle_for(&link).await.err().unwrap(),
    ] {
        assert!(
            error.to_string().contains("does not exist"),
            "the refusal must name the missing file: {error}"
        );
    }
    let names = std::fs::read_dir(directory.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    assert_eq!(names, vec![std::ffi::OsString::from("link.db")]);
    assert!(!link.exists(), "the link must stay dangling");
}

/// An in-memory database has no WAL and no file: neither entry may create a
/// lease file for it, and the refusal is not a live writer.
#[tokio::test]
async fn an_in_memory_database_is_refused_without_a_lease_file() {
    let memory = Path::new(":memory:");
    for error in [
        checkpoint::run_once(memory, 0).await.unwrap_err(),
        checkpoint::handle_for(memory).await.err().unwrap(),
        maintenance::checkpoint(memory.to_path_buf())
            .await
            .unwrap_err(),
    ] {
        assert!(!is_writers_active(&error), "{error}");
    }
    assert!(
        checkpoint::run_once(memory, 0)
            .await
            .unwrap_err()
            .to_string()
            .contains("in-memory"),
        "the checkpoint refusal must name the cause"
    );
    assert!(
        !maintenance_lock::lock_path(memory).exists(),
        "an in-memory database must not grow a disk lease"
    );
}
