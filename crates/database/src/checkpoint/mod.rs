//! Periodic passive WAL checkpoints keep write-ahead logs bounded while
//! writers stay online. One resident handle per database path and process
//! owns a single dedicated connection, opened while the pool is still
//! quiescent and reused for every pass; only a failed pass discards it, and
//! the next pass is the one case that calls `Database::connect()` again. Each
//! pass runs `PRAGMA wal_checkpoint(PASSIVE)` under a short busy timeout:
//! contention means skip without blocking, and `wal_autocheckpoint` still
//! bounds the WAL between passes. The strict TRUNCATE variant stays a
//! CLI-only operation ([`crate::maintenance::checkpoint`]).
//!
//! Every writer pool of a file publishes its own handle and runs the
//! periodic pass, in every process that has such a pool: passes of several
//! processes may overlap, the engine's WAL locks serialise them and a busy
//! pass is skipped. A process without a live writer pool has no resident
//! handle and its loop runs nothing. A handle is bound to the pool that
//! published it through a weak reference: a dropped pool disarms its handle at
//! once, so a retained handle never checkpoints again. The standalone
//! [`run_once`] entry reuses the resident handle of a live pool or opens a
//! private engine under the database's shared maintenance lease
//! ([`crate::maintenance_lock`]) -- one more writer, refused by nothing that
//! is a writer, waiting out an exclusive lease that rewrites the file.

mod handle;
mod periodic;

pub use handle::{checkpoint_passive, run_once_handle, CheckpointHandle, CheckpointStats};
pub use periodic::{periodic_pass, start_periodic, CheckpointPolicy, Outcome};

use anyhow::Result;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

/// Never wait longer than this for a contended checkpoint attempt: the
/// periodic pass skips instead of blocking application traffic.
pub(crate) const BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Standalone database opens performed by `handle_for`, counted per database
/// path. Handles published through `register` never increment it, so a stable
/// count for one path proves its resident pool handle was reused rather than
/// the file reopened (a process-wide total would move under unrelated tests).
fn standalone_open_counts() -> &'static Mutex<HashMap<PathBuf, usize>> {
    static OPENS: OnceLock<Mutex<HashMap<PathBuf, usize>>> = OnceLock::new();
    OPENS.get_or_init(Default::default)
}

/// Process-wide registry of resident handles keyed by database path. The
/// std mutex is only held for map reads and writes, never across an await.
fn handles() -> &'static Mutex<HashMap<PathBuf, Arc<CheckpointHandle>>> {
    static HANDLES: OnceLock<Mutex<HashMap<PathBuf, Arc<CheckpointHandle>>>> = OnceLock::new();
    HANDLES.get_or_init(Default::default)
}

/// Publish a resident handle under its own path and return the shared
/// handle. Registering the same path again replaces the previous entry, so
/// followers (the periodic loop) pick up a re-opened pool handle.
pub fn register(handle: CheckpointHandle) -> Arc<CheckpointHandle> {
    let handle = Arc::new(handle);
    handles()
        .lock()
        .expect("checkpoint handle registry poisoned")
        .insert(handle.path().to_path_buf(), Arc::clone(&handle));
    handle
}

/// Keep the handle already registered under this path, if any. The pool's
/// registry-hit open calls this so an already-primed handle is not replaced
/// by an unprimed one for the same engine.
pub fn register_if_absent(handle: CheckpointHandle) {
    handles()
        .lock()
        .expect("checkpoint handle registry poisoned")
        .entry(handle.path().to_path_buf())
        .or_insert_with(|| Arc::new(handle));
}

/// The resident handle registered for `db`, if any.
fn registered(db: &Path) -> Option<Arc<CheckpointHandle>> {
    handles()
        .lock()
        .expect("checkpoint handle registry poisoned")
        .get(db)
        .cloned()
}

/// Drop the registry entry while it still points at `handle`. A handle whose
/// pool is gone must not keep a dead engine resident for the rest of the
/// process, and a late observer must not remove a re-registered handle for
/// the same path.
pub(crate) fn prune(handle: &Arc<CheckpointHandle>) {
    let mut handles = handles()
        .lock()
        .expect("checkpoint handle registry poisoned");
    let stale = handles
        .get(handle.path())
        .is_some_and(|current| Arc::ptr_eq(current, handle));
    if stale {
        handles.remove(handle.path());
    }
}

/// Registry key for `db`: the canonical absolute path when the file exists
/// (the same key the pool registers under), otherwise the path as given.
pub(crate) fn lookup_key(db: &Path) -> PathBuf {
    std::path::absolute(db)
        .ok()
        .filter(|absolute| absolute.exists())
        .and_then(|absolute| absolute.canonicalize().ok())
        .unwrap_or_else(|| db.to_path_buf())
}

/// Standalone database opens performed so far by `handle_for` for `db`. Pool
/// handles published through `register` do not increment it, so a stable
/// count proves the resident handle was reused rather than the file reopened.
pub fn standalone_opens(db: &Path) -> usize {
    standalone_open_counts()
        .lock()
        .expect("standalone open counter poisoned")
        .get(&lookup_key(db))
        .copied()
        .unwrap_or(0)
}

/// Per-path build gates for tests of the standalone open. A gate is armed
/// before `handle_for` runs and holds the executor-owned build after it has
/// taken the maintenance lease and before it builds the writable engine, so a
/// test can cancel the caller while the detached build still owns the lease.
#[cfg(test)]
fn build_gates() -> &'static Mutex<HashMap<PathBuf, Arc<tokio::sync::Semaphore>>> {
    static GATES: OnceLock<Mutex<HashMap<PathBuf, Arc<tokio::sync::Semaphore>>>> = OnceLock::new();
    GATES.get_or_init(Default::default)
}

/// Arm the build gate for `path` and return the semaphore that releases it.
#[cfg(test)]
pub(crate) fn arm_build_gate(path: &Path) -> Arc<tokio::sync::Semaphore> {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    build_gates()
        .lock()
        .expect("checkpoint build gate registry poisoned")
        .insert(path.to_path_buf(), Arc::clone(&gate));
    gate
}

/// Hold the build of `path` at its gate, if one is armed. The gate is consumed
/// by the first build that reaches it, so only the armed open ever waits here.
#[cfg(test)]
async fn wait_at_build_gate(path: &Path) {
    let gate = build_gates()
        .lock()
        .expect("checkpoint build gate registry poisoned")
        .remove(path);
    if let Some(gate) = gate {
        gate.acquire().await.expect("build gate closed").forget();
    }
}

/// The handle a checkpoint pass for `db` must use.
///
/// The resident handle is reused while the pool that published it is still
/// alive; a handle left behind by a dropped pool is pruned and never
/// checkpointed. Otherwise the pass takes the database's shared maintenance
/// lease and opens a private engine under it: the pass is one more writer for
/// as long as it lives, so a TRUNCATE checkpoint or a writable bridge running
/// meanwhile fails fast with `WritersActive` instead of rewriting the WAL
/// under it, while writer pools here or in other processes open beside it as
/// they always do; an exclusive lease held by offline maintenance is waited
/// out within the open's budget. The standalone handle is never registered: a
/// resident standalone engine would outlive the pass it was built for.
///
/// The lease is taken INSIDE the executor-owned open, together with the engine
/// it guards: a caller cancelled at any await only detaches that task, which
/// keeps the lease until the engine it built is gone. The lease can therefore
/// never be released by a cancelled caller while a writable engine is still
/// being built or still exists, and a detached open leaves no engine behind.
pub async fn handle_for(db: &Path) -> Result<Arc<CheckpointHandle>> {
    let key = lookup_key(db);
    // The first two refusals happen before any artifact is created: an
    // in-memory database has no WAL to merge, and a checkpoint never creates
    // a database.
    anyhow::ensure!(
        key != Path::new(":memory:"),
        "an in-memory database has no write-ahead log to checkpoint"
    );
    anyhow::ensure!(
        key.is_file(),
        "database file does not exist: {}",
        key.display()
    );
    // Third refusal before any artifact: the engine takes a string path, so
    // one that is not UTF-8 can never be checkpointed; refusing it here keeps
    // the standalone open from taking the lease for it (`engine::open_writer`
    // refuses it again before the lease).
    crate::engine::engine_name(&key)?;
    if let Some(handle) = registered(&key) {
        if handle.owner_alive() {
            return Ok(handle);
        }
        prune(&handle);
    }
    let path = key.clone();
    // The lease is taken before the engine opens the file, and both live in
    // this task: the lease is dropped only after the engine it was taken for,
    // whether the task returns to a waiting caller or finishes detached after
    // that caller was cancelled.
    let (database, lease) = crate::runtime::run(true, async move {
        // A writer open like any other: the lease is taken before the engine
        // opens the file, beside every other writer of the file.
        #[cfg(test)]
        let before_build = || wait_at_build_gate(&path);
        #[cfg(not(test))]
        let before_build = || async {};
        let (database, lease) = crate::engine::open_writer(&path, &path, before_build).await?;
        // Counted under the lease, before the engine is handed over: a caller
        // cancelled mid-open still leaves exactly this one open behind.
        *standalone_open_counts()
            .lock()
            .expect("standalone open counter poisoned")
            .entry(path.clone())
            .or_insert(0) += 1;
        // Tuple fields drop in order: a detached task drops the engine
        // before it releases the lease that guarded it.
        Ok((database, lease))
    })
    .await?;
    let handle = Arc::new(CheckpointHandle::standalone(key, database, lease));
    // Nothing else holds this standalone engine yet: the one connect() of
    // the handle happens here, before any pass.
    handle.prime().await?;
    Ok(handle)
}

/// One gated pass for `db`.
///
/// The resident handle of a live pool is reused, so the live writer is never
/// opened a second time. Without such a pool the pass takes the shared
/// maintenance lease and opens a private engine under it, like any other
/// writer of the file. A missing file is refused before the lease file is
/// created: a checkpoint never creates a database.
pub async fn run_once(db: &Path, threshold_bytes: u64) -> Result<Outcome> {
    let handle = handle_for(db).await?;
    run_once_handle(&handle, threshold_bytes).await
}

/// WAL companion path as laid out by the engine: `{database}-wal`. The
/// suffix is appended to the path's own OS string, so a name that is not
/// UTF-8 keeps its exact companion name instead of a lossy rendering.
pub fn wal_path(db: &Path) -> PathBuf {
    let mut name = db.as_os_str().to_os_string();
    name.push("-wal");
    PathBuf::from(name)
}

/// Current WAL size in bytes; an absent WAL counts as zero.
pub fn wal_size(db: &Path) -> u64 {
    std::fs::metadata(wal_path(db))
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

#[cfg(test)]
mod cancellation_tests;
