//! A database path that is not UTF-8 can never reach the engine, which takes
//! a string path. Every entry point that would take a lease, record a marker
//! or open an engine for such a path refuses it first, so nothing is left
//! behind: no lease file, no engine marker, no WAL coordination companion,
//! no database file. The companion paths themselves extend the raw bytes of
//! the path, never a lossy rendering of it. (The writable and read-only
//! bridge is covered by its unit test: it refuses before taking a lease.)
#![cfg(unix)]
mod support;

use opencoder_database::checkpoint::{self, wal_path};
use opencoder_database::maintenance_lock::lock_path;
use opencoder_database::{maintenance, Database};
use std::path::Path;
use support::{entries, non_utf8_database, non_utf8_directory};

const UTF8: &str = "database path must be UTF-8";

/// Sorted entry names of the database's own directory; the database file is
/// the only entry the fixture created.
fn only_the_database(database: &Path) -> Vec<std::ffi::OsString> {
    vec![database.file_name().unwrap().to_os_string()]
}

/// Companion paths extend the path's own bytes: a name that is not UTF-8
/// keeps its exact lease file and WAL companion names.
#[test]
fn companion_paths_extend_the_bytes_of_a_non_utf8_path() {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let name = std::ffi::OsString::from_vec(b"data/invalid-\xff.db".to_vec());
    let database = Path::new(&name);
    assert_eq!(
        lock_path(database).as_os_str().as_bytes(),
        b"data/invalid-\xff.db.maintenance.lock",
        "the lease file name must extend the raw path bytes"
    );
    assert_eq!(
        wal_path(database).as_os_str().as_bytes(),
        b"data/invalid-\xff.db-wal",
        "the WAL companion name must extend the raw path bytes"
    );
}

/// A writer or reader open of an existing file under a non-UTF-8 path is
/// refused with the cause, before the marker is recorded, before a lease is
/// taken and before the engine touches the file.
#[tokio::test]
async fn opens_refuse_a_non_utf8_path_before_any_artifact() {
    let Some((directory, database)) = non_utf8_database() else {
        return;
    };
    let key = database.canonicalize().unwrap();
    assert!(
        key.to_str().is_none(),
        "fixture: the path must stay non-UTF-8"
    );
    for refused in [
        Database::open(&database)
            .await
            .err()
            .expect("a writer open must refuse a non-UTF-8 path"),
        Database::open_reader(&database)
            .await
            .err()
            .expect("a reader open must refuse a non-UTF-8 path"),
    ] {
        assert!(
            refused.to_string().contains(UTF8),
            "the refusal must name the cause: {refused:#}"
        );
    }
    assert!(
        !lock_path(&key).exists(),
        "no lease file may be left behind"
    );
    assert_eq!(
        entries(directory.path()),
        only_the_database(&database),
        "no marker, lease or companion may appear beside the database"
    );
}

/// The same refusal for a database that does not exist yet: a non-UTF-8
/// path is refused before anything is created under it, by either intent.
#[tokio::test]
async fn opens_create_nothing_under_a_non_utf8_path() {
    let Some((_parent, directory)) = non_utf8_directory() else {
        return;
    };
    assert!(
        directory.to_str().is_none(),
        "fixture: the path must stay non-UTF-8"
    );
    let database = directory.join("new.db");
    for refused in [
        Database::open(&database)
            .await
            .err()
            .expect("a writer open must refuse a non-UTF-8 path"),
        Database::open_reader(&database)
            .await
            .err()
            .expect("a reader open must refuse a non-UTF-8 path"),
    ] {
        assert!(
            refused.to_string().contains(UTF8),
            "the refusal must name the cause: {refused:#}"
        );
    }
    assert_eq!(
        entries(&directory),
        Vec::<std::ffi::OsString>::new(),
        "no database, marker, lease or companion may be created"
    );
}

/// The maintenance and checkpoint entries refuse the path the same way: the
/// TRUNCATE checkpoint before it takes the exclusive lease, the standalone
/// checkpoint pass before it takes its own lease or builds an engine,
/// inspection before the engine opens, and backup before any copy exists.
#[tokio::test]
async fn maintenance_entries_refuse_a_non_utf8_path_before_any_artifact() {
    let Some((directory, database)) = non_utf8_database() else {
        return;
    };
    let key = database.canonicalize().unwrap();
    assert!(
        key.to_str().is_none(),
        "fixture: the path must stay non-UTF-8"
    );
    let output = directory.path().join("copy.db");
    for refused in [
        maintenance::checkpoint(database.clone()).await.unwrap_err(),
        checkpoint::run_once(&database, 0).await.unwrap_err(),
        maintenance::query(database.clone(), "SELECT 1".into(), vec![])
            .await
            .unwrap_err(),
        maintenance::backup(database.clone(), output.clone())
            .await
            .unwrap_err(),
    ] {
        assert!(
            refused.to_string().contains(UTF8),
            "the refusal must name the cause: {refused:#}"
        );
    }
    assert_eq!(checkpoint::standalone_opens(&database), 0);
    assert!(
        !lock_path(&key).exists(),
        "no lease file may be left behind"
    );
    assert!(!output.exists(), "no backup copy may appear");
    assert_eq!(
        entries(directory.path()),
        only_the_database(&database),
        "no staging directory or other entry may be left behind"
    );
}
