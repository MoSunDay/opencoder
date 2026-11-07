//! Shared RocksDB engine wrapper used by every rdb-backed store
//! (`RdbStore`, `FleetStore`, `TsRegistry`).
//!
//! It is the moral equivalent of `opencoder_database::Database`: a thin
//! handle over `rdb::store::Store` plus the exclusive write lease that
//! emulates SQLite's `BEGIN IMMEDIATE` single-writer discipline. All state
//! is shared via `Arc`; the handle itself is a plain `Clone` carrier, not
//! an encapsulated object graph.
//!
//! Conventions:
//! - `write` takes the lease, then runs the (blocking) closure on the
//!   tokio blocking pool. Closures receive `&rdb::store::Store` and must
//!   apply their changes through one atomic `batch_write` so a failure
//!   leaves nothing behind (rollback semantics).
//! - `read` runs the blocking closure without the lease; RocksDB
//!   iterators are consistent snapshots per operation.
//! - Closures own their inputs (`move`) because they cross threads.

use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex, OnceLock, Weak};

use anyhow::{anyhow, Result};
use tokio::sync::Mutex;

/// One open RocksDB database plus the write lease that all handles to the
/// same directory share. RocksDB takes an OS-level lock on its directory, so
/// a second `rdb::store::open` on the same path fails; the process-wide
/// registry below hands every `RdbDb::open` of a live path the same core.
/// That mirrors the SQL backend's multi-connection sharing while keeping the
/// single-writer invariant across handles (one lease per database). Before
/// touching RocksDB, a fresh core also acquires the cross-process
/// [`OwnerLock`] on a sibling lock file so contention from another process
/// surfaces as a clear error naming the database and the recorded owner.
struct RdbCore {
    store: Arc<rdb::store::Store>,
    write_mu: Mutex<()>,
    /// Held for the core's lifetime; released by `Drop` (and by the OS when
    /// the process dies).
    #[allow(dead_code)] // RAII: dropping the field releases the cross-process lock.
    owner: OwnerLock,
}

/// Cross-process advisory ownership lock over one database directory: an
/// exclusive `flock` on the sibling `<db-dir>.owner.lock` file. RocksDB's own
/// directory lock already refuses a second process, but with an opaque error;
/// this lock is taken first and records non-sensitive owner metadata (pid +
/// process description only — never tokens, secrets or environment values)
/// so a busy database can be attributed.
struct OwnerLock {
    file: File,
}

impl Drop for OwnerLock {
    fn drop(&mut self) {
        if let Err(error) = fs2::FileExt::unlock(&self.file) {
            tracing::warn!(%error, "rdb owner lock release failed");
        }
    }
}

/// Sibling advisory-lock file for a database directory: the database's file
/// name plus `.owner.lock` in the same parent (`opencoder.db.rdb` →
/// `opencoder.db.rdb.owner.lock`).
fn owner_lock_path(db_dir: &Path) -> PathBuf {
    let mut name = db_dir
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".owner.lock");
    db_dir.with_file_name(name)
}

/// Non-sensitive owner line written into the held lock file: the owning pid
/// plus the executable's file name as a human-readable process description.
fn owner_metadata() -> String {
    let proc = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "unknown".to_string());
    format!("pid={} proc={proc}\n", std::process::id())
}

/// Acquire the exclusive ownership lock for `db_dir`, or fail with an error
/// that names the database path and the recorded holder (if any).
fn acquire_owner_lock(db_dir: &Path) -> Result<OwnerLock> {
    let lock_path = owner_lock_path(db_dir);
    if let Some(parent) = lock_path.parent() {
        // Best-effort: the store open below reports a real failure anyway.
        std::fs::create_dir_all(parent).ok();
    }
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|error| anyhow!("open rdb owner lock {}: {error}", lock_path.display()))?;
    if let Err(error) = fs2::FileExt::try_lock_exclusive(&file) {
        if error.kind() != std::io::ErrorKind::WouldBlock {
            return Err(anyhow!(
                "lock rdb owner file {}: {error}",
                lock_path.display()
            ));
        }
        let holder = std::fs::read_to_string(&lock_path)
            .ok()
            .and_then(|text| text.lines().next().map(str::to_string))
            .filter(|line| !line.trim().is_empty())
            .unwrap_or_else(|| "no owner metadata recorded".to_string());
        return Err(anyhow!(
            "rdb database at {} is already owned by another process \
             (owner lock {} is held; {holder})",
            db_dir.display(),
            lock_path.display()
        ));
    }
    // Refresh the holder metadata while we own the lock; identification only,
    // so a write failure never blocks opening the database.
    {
        use std::io::Write as _;
        let _ = file
            .set_len(0)
            .and_then(|()| file.write_all(owner_metadata().as_bytes()))
            .and_then(|()| file.sync_all());
    }
    Ok(OwnerLock { file })
}

fn registry() -> &'static StdMutex<HashMap<PathBuf, Weak<RdbCore>>> {
    static REGISTRY: OnceLock<StdMutex<HashMap<PathBuf, Weak<RdbCore>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| StdMutex::new(HashMap::new()))
}

/// Plain data carrier: engine handle + write lease. No domain logic.
#[derive(Clone)]
pub struct RdbDb {
    core: Arc<RdbCore>,
}

impl RdbDb {
    /// Open (or create) a RocksDB database directory at `path`.
    ///
    /// A path that is already open in this process (and still has a live
    /// handle) is shared instead of reopened, preserving RocksDB's directory
    /// lock and the single-writer lease. All handles drop -> the database
    /// closes and a later open sees the on-disk state.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if path.exists() && !path.is_dir() {
            return Err(anyhow!(
                "rdb database path {} exists and is not a directory; a RocksDB \
                 store needs a directory (a legacy SQLite file must be migrated, \
                 not opened)",
                path.display()
            ));
        }
        std::fs::create_dir_all(path)
            .map_err(|error| anyhow!("create rdb directory {}: {error}", path.display()))?;
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let mut reg = registry()
            .lock()
            .map_err(|_| anyhow!("rdb handle registry poisoned"))?;
        // Same-process reuse first: a live core already holds the owner lock
        // and the RocksDB directory lock for this path.
        if let Some(core) = reg.get(&canonical).and_then(Weak::upgrade) {
            return Ok(Self { core });
        }
        let owner = acquire_owner_lock(&canonical)?;
        let display = canonical
            .to_str()
            .ok_or_else(|| anyhow!("storage path is not valid UTF-8"))?
            .to_owned();
        let store =
            rdb::store::open(&display).map_err(|e| anyhow!("open rdb store at {display}: {e}"))?;
        let core = Arc::new(RdbCore {
            store: Arc::new(store),
            write_mu: Mutex::new(()),
            owner,
        });
        reg.insert(canonical, Arc::downgrade(&core));
        reg.retain(|_, weak| weak.strong_count() > 0);
        Ok(Self { core })
    }

    /// Raw engine handle for direct free-function calls (reads, iterators).
    pub fn handle(&self) -> &rdb::store::Store {
        &self.core.store
    }

    /// Shared handle clone (for `spawn_blocking` closures).
    pub fn shared(&self) -> Arc<rdb::store::Store> {
        Arc::clone(&self.core.store)
    }

    /// Exclusive-writer batch execution ≈ `BEGIN IMMEDIATE` … `COMMIT`.
    pub async fn write<T, F>(&self, work: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&rdb::store::Store) -> Result<T> + Send + 'static,
    {
        let _lease = self.core.write_mu.lock().await;
        let store = self.shared();
        tokio::task::spawn_blocking(move || work(&store))
            .await
            .map_err(|e| anyhow!("storage writer panicked: {e}"))?
    }

    /// Lease-free blocking read (single RocksDB snapshot semantics).
    pub async fn read<T, F>(&self, work: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&rdb::store::Store) -> Result<T> + Send + 'static,
    {
        let store = self.shared();
        tokio::task::spawn_blocking(move || work(&store))
            .await
            .map_err(|e| anyhow!("storage reader panicked: {e}"))?
    }
}

#[cfg(test)]
mod tests {
    use super::{acquire_owner_lock, owner_lock_path, registry, RdbDb};

    /// Same-process opens of a live path share one core (and therefore one
    /// owner lock) instead of contending with each other.
    #[test]
    fn same_process_opens_share_one_core() {
        let dir = tempfile::tempdir().unwrap();
        let first = RdbDb::open(dir.path()).expect("first open");
        let second = RdbDb::open(dir.path()).expect("second open reuses the core");
        let canonical = dir.path().canonicalize().unwrap();
        let reg = registry().lock().unwrap();
        let weak = reg
            .get(&canonical)
            .expect("live core stays registered")
            .clone();
        // Both handles plus the registry insertion keep the core alive; a
        // fresh open must have upgraded the same entry, not locked a second
        // core into existence (which would have failed on the owner lock).
        assert!(
            weak.strong_count() >= 2,
            "second open reused the registered core"
        );
        drop((first, second));
    }

    /// A foreign exclusive lock on the sibling owner file produces a busy
    /// error that names the database path and the recorded holder.
    #[test]
    fn manually_locked_owner_file_yields_busy_error() {
        let tmp = tempfile::tempdir().unwrap();
        let db_dir = tmp.path().join("opencoder.db.rdb");
        let lock_path = owner_lock_path(&db_dir);
        let mut holder = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&lock_path)
            .unwrap();
        use std::io::Write as _;
        holder
            .write_all(b"pid=424242 proc=opencoder-holder\n")
            .unwrap();
        fs2::FileExt::try_lock_exclusive(&holder).unwrap();

        let error = match RdbDb::open(&db_dir) {
            Err(error) => error.to_string(),
            Ok(_) => panic!("opening a locked rdb directory must fail"),
        };
        assert!(
            error.contains(&db_dir.display().to_string()),
            "busy error must name the database path: {error}"
        );
        assert!(
            error.contains("424242") && error.contains("opencoder-holder"),
            "busy error must surface the recorded holder: {error}"
        );

        drop(holder);
        // Once the foreign lock is gone the same path opens normally.
        RdbDb::open(&db_dir).expect("open succeeds after lock release");
    }

    /// The ownership lock lives with the core: dropping the last handle
    /// releases it, so another opener (here: a manual lock) can take over.
    #[test]
    fn owner_lock_releases_with_last_handle() {
        let tmp = tempfile::tempdir().unwrap();
        let db_dir = tmp.path().join("db");
        {
            let db = RdbDb::open(&db_dir).unwrap();
            drop(db);
        }
        let lock_path = owner_lock_path(&db_dir);
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock_path)
            .unwrap();
        assert!(
            fs2::FileExt::try_lock_exclusive(&file).is_ok(),
            "owner lock must be free after the last handle drops"
        );
    }

    /// `acquire_owner_lock` itself returns the busy error (and the metadata
    /// it writes is non-sensitive: pid + process description only).
    #[test]
    fn acquire_writes_non_sensitive_owner_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        let db_dir = tmp.path().join("db");
        let lock = acquire_owner_lock(&db_dir).expect("fresh lock acquires");
        let text = std::fs::read_to_string(owner_lock_path(&db_dir)).unwrap();
        assert!(text.contains("pid="), "metadata records the pid: {text}");
        drop(lock);
    }

    #[test]
    fn open_rejects_a_plain_file_path_with_a_clear_error() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("opencoder.db");
        std::fs::write(&file, b"legacy sqlite bytes").unwrap();
        let error = match RdbDb::open(&file) {
            Err(error) => error.to_string(),
            Ok(_) => panic!("opening a plain file path must fail"),
        };
        assert!(error.contains("is not a directory"), "{error}");
        assert!(error.contains("opencoder.db"), "{error}");
    }
}
