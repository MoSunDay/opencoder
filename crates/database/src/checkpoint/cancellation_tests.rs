//! Cancellation of the standalone open: the maintenance lease belongs to the
//! executor-owned build and is released only with the engine it guards.
use super::*;

/// Create `path` through a writer pool and release the pool, so the file and
/// its WAL coordination companion exist and no lease of this process is held.
async fn released_database(directory: &Path) -> PathBuf {
    let path = directory.join("cancelled.db");
    drop(
        crate::Database::open(&path)
            .await
            .expect("create the standalone test database"),
    );
    path
}

/// Whether offline maintenance could take the exclusive lease of `key` right
/// now: true exactly while no writer pool, standalone pass or writable bridge
/// holds the shared lease.
fn lease_free(key: &Path) -> bool {
    crate::maintenance_lock::try_acquire_exclusive(key).is_ok()
}

/// Cancelling `handle_for` after its build took the shared lease but before
/// the engine was built must not release that lease: the detached build owns
/// both the lease and the engine it is about to create, a TRUNCATE checkpoint
/// fails fast meanwhile, and the lease is released only after the engine is
/// gone.
#[tokio::test]
async fn a_cancelled_standalone_open_keeps_its_lease_with_the_build() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = released_database(directory.path()).await;
    let key = path.canonicalize().expect("canonical database path");
    assert!(lease_free(&key), "the dropped pool left no writer behind");

    // Hold the executor-owned build right after it takes the lease, before it
    // opens the writable engine.
    let gate = arm_build_gate(&key);
    let task_key = key.clone();
    let caller = tokio::spawn(async move { handle_for(&task_key).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while lease_free(&key) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the build must take the lease");
    assert!(
        !caller.is_finished(),
        "the gate keeps the build from completing"
    );

    // Cancel the caller after the lease is taken and before build completion.
    caller.abort();
    assert!(
        caller
            .await
            .err()
            .expect("the caller must not complete")
            .is_cancelled(),
        "the caller was cancelled"
    );

    // The detached build still owns the lease: offline maintenance fails fast
    // with a live-writer conflict, and no engine has been opened for it yet.
    assert!(
        !lease_free(&key),
        "a TRUNCATE checkpoint must see the building writer"
    );
    assert!(
        crate::maintenance_lock::is_writers_active(
            &crate::maintenance::checkpoint(path.clone())
                .await
                .expect_err("TRUNCATE must fail fast beside the building writer")
        ),
        "the conflict is a live writer"
    );
    // The counter is still at its value before the gated build finishes.
    let opens = standalone_opens(&path);

    // Let the detached build finish: it opens exactly one engine, drops it
    // and only then releases the lease.
    gate.add_permits(1);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !lease_free(&key) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the finished build must release its lease");
    assert_eq!(
        standalone_opens(&path),
        opens + 1,
        "the detached build opened one engine under its lease and left nothing behind"
    );

    // No engine survived the cancelled open: a fresh standalone open takes its
    // own shared lease and is visible to offline maintenance for exactly as
    // long as it lives.
    let handle = handle_for(&key).await.expect("the path is free again");
    assert!(handle.owner_alive(), "the fresh handle owns its lease");
    assert!(
        !lease_free(&key),
        "the standalone pass is one more writer for its whole life"
    );
    assert_eq!(
        standalone_opens(&path),
        opens + 2,
        "one more open for the fresh handle"
    );
    drop(handle);
    assert!(
        lease_free(&key),
        "dropping the standalone handle releases its lease with the engine"
    );
    assert_eq!(
        periodic_pass(&key, 0)
            .await
            .expect("no resident standalone handle"),
        None,
        "no standalone engine may stay resident after its handle is gone"
    );
}
