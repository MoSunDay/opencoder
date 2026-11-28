//! The per-database maintenance lease: `<db>.maintenance.lock`.
//!
//! Writers never exclude each other: any number of writable pools, in this
//! process or any other, open the same file through the engine's multi-process
//! shared WAL and commit beside each other, serialised by the engine's own WAL
//! writer lock. What must not overlap a writer is offline maintenance that
//! rewrites the file in place -- the TRUNCATE checkpoint and the writable
//! maintenance bridge. Those take an EXCLUSIVE lease and fail fast with
//! [`WritersActive`] ("writers still active") while any writer is alive, and
//! every writable pool holds a SHARED lease for its whole lifetime, so a live
//! writer is exactly a held shared lease. A standalone checkpoint pass is one
//! more writer and leases its database in the same way.
//!
//! The other direction is symmetric: a writer that arrives while maintenance
//! holds the exclusive lease cannot take a shared one and reports
//! [`MaintenanceActive`], which the engine retries for a bounded budget before
//! giving up. Readers take no lease at all, so a TRUNCATE that a reader's
//! snapshot keeps busy reports the engine's own busy text, never a live writer.
//!
//! The lock is per open file description, so a second lease in this process is
//! subject to the same rule as one in another process. The lease file is never
//! deleted: it carries no state beyond the flock, and unlinking it would let a
//! second file take over the name while the first one is still held.

use anyhow::{Context, Result};
use fs2::FileExt;
use std::{
    fmt,
    fs::{File, OpenOptions},
    io,
    path::{Path, PathBuf},
};

/// Suffix appended to the database path for its lease file.
pub const LOCK_SUFFIX: &str = ".maintenance.lock";

/// Lease file path for `db`: `<db>.maintenance.lock` next to the database.
///
/// The suffix is appended to the path's own OS string, never to a rendered
/// form of it: a name that is not UTF-8 keeps its exact bytes instead of a
/// lossy rendering. The engine cannot open such a path, so [`acquire`] refuses
/// it before any lease file appears.
pub fn lock_path(db: &Path) -> PathBuf {
    let mut name = db.as_os_str().to_os_string();
    name.push(LOCK_SUFFIX);
    PathBuf::from(name)
}

/// A live writer holds the database's shared lease, so offline maintenance
/// cannot take the exclusive one. Carried through `anyhow::Error` so callers
/// can branch with [`is_writers_active`].
#[derive(Debug)]
pub struct WritersActive {
    pub database: PathBuf,
    pub lock: PathBuf,
}

impl fmt::Display for WritersActive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "writers still active: database {} has active writers (maintenance lease {}); stop \
             every writer before offline maintenance",
            self.database.display(),
            self.lock.display()
        )
    }
}

impl std::error::Error for WritersActive {}

/// Offline maintenance holds the database's exclusive lease, so a writer
/// cannot take a shared one. Carried through `anyhow::Error` so the engine can
/// wait the maintenance out with [`is_maintenance_active`].
#[derive(Debug)]
pub struct MaintenanceActive {
    pub database: PathBuf,
    pub lock: PathBuf,
}

impl fmt::Display for MaintenanceActive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "offline maintenance holds {} exclusively (maintenance lease {})",
            self.database.display(),
            self.lock.display()
        )
    }
}

impl std::error::Error for MaintenanceActive {}

/// Whether `error` reports a lease conflict, looked up anywhere in the chain.
///
/// Both directions count: maintenance beside a live writer ([`WritersActive`])
/// and a writer beside maintenance ([`MaintenanceActive`]). Callers that wait
/// for maintenance to finish distinguish the second with
/// [`is_maintenance_active`]; anything else -- an unreadable lease file, a
/// busy WAL, a plain failure -- keeps its own meaning.
pub fn is_writers_active(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause.downcast_ref::<WritersActive>().is_some()
            || cause.downcast_ref::<MaintenanceActive>().is_some()
    })
}

/// Whether `error` reports offline maintenance holding the exclusive lease.
pub fn is_maintenance_active(error: &anyhow::Error) -> bool {
    error
        .chain()
        .any(|cause| cause.downcast_ref::<MaintenanceActive>().is_some())
}

/// How a lease is held. Shared leases coexist with each other and exclude an
/// exclusive one; an exclusive lease excludes every other lease.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// A writable pool or a standalone checkpoint pass.
    Shared,
    /// Offline maintenance: the TRUNCATE checkpoint, the writable bridge.
    Exclusive,
}

/// A held maintenance lease: it exists only while the lock is held, and
/// dropping it releases the lock.
#[derive(Debug)]
pub struct MaintenanceLease {
    /// The open file description the lock lives on; nothing reads it.
    file: File,
    lock_path: PathBuf,
    database: PathBuf,
    mode: Mode,
}

impl MaintenanceLease {
    /// The database this lease guards, as the caller named it.
    pub fn database(&self) -> &Path {
        &self.database
    }

    /// The lease file next to the database.
    pub fn lock_path(&self) -> &Path {
        &self.lock_path
    }

    /// Shared (a writer) or exclusive (offline maintenance).
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Turn an exclusive lease into a shared one, atomically, on the same open
    /// file description. The first opener of a file holds the exclusive lease
    /// only while it creates and initialises the file and its WAL coordination
    /// companion; downgrading (rather than dropping and re-acquiring) leaves no
    /// window in which maintenance could slip in before other writers join.
    pub(crate) fn downgrade(&mut self) -> Result<()> {
        anyhow::ensure!(
            self.mode == Mode::Exclusive,
            "only an exclusive lease can be downgraded"
        );
        FileExt::lock_shared(&self.file).with_context(|| {
            format!(
                "downgrade database maintenance lease {}",
                self.lock_path.display()
            )
        })?;
        self.mode = Mode::Shared;
        Ok(())
    }
}

impl Drop for MaintenanceLease {
    fn drop(&mut self) {
        // Closing the file would release the lock on its own; unlocking first
        // keeps the release independent of any other handle to the same open
        // file description.
        let _ = FileExt::unlock(&self.file);
    }
}

/// Open (creating if needed) the lease file, private to the user.
fn open_lock_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

/// The conflict the loser of a lease acquisition reports: asking for the
/// exclusive lease finds live writers, asking for a shared one finds
/// maintenance.
fn conflict(database: &Path, lock: &Path, requested: Mode) -> anyhow::Error {
    let (database, lock) = (database.to_path_buf(), lock.to_path_buf());
    match requested {
        Mode::Exclusive => anyhow::Error::new(WritersActive { database, lock }),
        Mode::Shared => anyhow::Error::new(MaintenanceActive { database, lock }),
    }
}

/// Take the lease of `database` in `mode`, without waiting.
fn acquire(database: &Path, mode: Mode) -> Result<MaintenanceLease> {
    anyhow::ensure!(
        database != Path::new(":memory:"),
        "an in-memory database takes no maintenance lease"
    );
    anyhow::ensure!(
        database.to_str().is_some(),
        "database path must be UTF-8: {}",
        database.display()
    );
    let lock = lock_path(database);
    let file = open_lock_file(&lock)
        .with_context(|| format!("open database maintenance lease {}", lock.display()))?;
    // fs2 reports contention as EWOULDBLOCK on every supported platform; any
    // other failure (unsupported lock, I/O error) is not a conflict. The
    // request never waits: a held lease means the caller fails fast, and the
    // engine is the one place that retries a shared acquisition.
    let locked = match mode {
        Mode::Shared => FileExt::try_lock_shared(&file),
        Mode::Exclusive => FileExt::try_lock_exclusive(&file),
    };
    locked.map_err(|error| match error.kind() {
        io::ErrorKind::WouldBlock => conflict(database, &lock, mode),
        _ => anyhow::Error::new(error).context(format!(
            "lock database maintenance lease {}",
            lock.display()
        )),
    })?;
    Ok(MaintenanceLease {
        file,
        lock_path: lock,
        database: database.to_path_buf(),
        mode,
    })
}

/// The lease a writable open (a pool, a standalone checkpoint pass) holds for
/// its whole lifetime. Never waits: [`MaintenanceActive`] while offline
/// maintenance holds the exclusive lease.
pub fn acquire_shared(database: &Path) -> Result<MaintenanceLease> {
    acquire(database, Mode::Shared)
}

/// The lease offline maintenance holds while it rewrites the file. Never
/// waits: [`WritersActive`] while any writer is alive.
pub fn try_acquire_exclusive(database: &Path) -> Result<MaintenanceLease> {
    acquire(database, Mode::Exclusive)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_path_sits_next_to_the_database() {
        assert_eq!(
            lock_path(Path::new("/data/app.db")),
            PathBuf::from("/data/app.db.maintenance.lock")
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_suffix_keeps_the_exact_bytes_of_a_non_utf8_name() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let directory = tempfile::tempdir().unwrap();
        let bytes = vec![b'd', b'b', 0xff];
        let database = directory.path().join(OsStr::from_bytes(&bytes));
        let mut expected = database.as_os_str().as_bytes().to_vec();
        expected.extend_from_slice(LOCK_SUFFIX.as_bytes());
        assert_eq!(
            lock_path(&database).as_os_str().as_bytes(),
            expected.as_slice()
        );
        assert!(acquire_shared(&database).is_err());
        assert!(!lock_path(&database).exists());
    }

    #[test]
    fn an_in_memory_database_takes_no_lease() {
        assert!(acquire_shared(Path::new(":memory:")).is_err());
        assert!(try_acquire_exclusive(Path::new(":memory:")).is_err());
    }

    #[test]
    fn a_downgraded_lease_admits_writers_and_still_excludes_maintenance() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("first.db");
        let mut lease = try_acquire_exclusive(&database).unwrap();
        assert_eq!(lease.mode(), Mode::Exclusive);
        lease.downgrade().unwrap();
        assert_eq!(lease.mode(), Mode::Shared);
        // Another writer joins immediately; maintenance still fails fast.
        let other = acquire_shared(&database).unwrap();
        let refused = try_acquire_exclusive(&database).unwrap_err();
        assert!(is_writers_active(&refused));
        assert!(refused
            .to_string()
            .contains(&database.display().to_string()));
        drop(other);
        drop(lease);
        let exclusive = try_acquire_exclusive(&database).unwrap();
        // And a writer arriving beside maintenance is refused the other way.
        let refused = acquire_shared(&database).unwrap_err();
        assert!(is_maintenance_active(&refused));
        assert!(is_writers_active(&refused));
        drop(exclusive);
        drop(acquire_shared(&database).unwrap());
    }
}
