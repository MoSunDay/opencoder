//! Fixtures shared by the database crate's integration test targets.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

/// A real, empty file under a name that is not valid UTF-8, alone in a
/// directory of its own, for the entry-point refusals. The default
/// temporary directory is preferred; a file system that only accepts UTF-8
/// names (`utf8only`) refuses the name there, so `/dev/shm` is tried next.
/// When neither accepts it the fixture prints why and returns `None`, so
/// the caller returns instead of failing over an input the platform cannot
/// express.
#[cfg(unix)]
pub fn non_utf8_database() -> Option<(tempfile::TempDir, PathBuf)> {
    use std::os::unix::ffi::OsStringExt;
    let name = std::ffi::OsString::from_vec(b"invalid-\xff.db".to_vec());
    let mut refusals = Vec::new();
    for root in [None, Some(Path::new("/dev/shm"))] {
        let directory = match root {
            None => tempfile::tempdir(),
            Some(root) => tempfile::Builder::new()
                .prefix("non-utf8-")
                .tempdir_in(root),
        };
        let directory = match directory {
            Ok(directory) => directory,
            Err(error) => {
                refusals.push(format!(
                    "{}: {error}",
                    root.unwrap_or(Path::new("<temporary directory>")).display()
                ));
                continue;
            }
        };
        let database = directory.path().join(&name);
        match std::fs::write(&database, b"") {
            Ok(()) => return Some((directory, database)),
            Err(error) => refusals.push(format!("{}: {error}", directory.path().display())),
        }
    }
    eprintln!(
        "skipping: no directory accepts a non-UTF-8 file name: {}",
        refusals.join("; ")
    );
    None
}

/// A directory of its own under a name that is not valid UTF-8, for
/// refusals of a database that does not exist yet; same fallbacks and
/// `None` semantics as [`non_utf8_database`].
#[cfg(unix)]
pub fn non_utf8_directory() -> Option<(tempfile::TempDir, PathBuf)> {
    use std::os::unix::ffi::OsStringExt;
    let name = std::ffi::OsString::from_vec(b"invalid-\xff".to_vec());
    let mut refusals = Vec::new();
    for root in [None, Some(Path::new("/dev/shm"))] {
        let parent = match root {
            None => tempfile::tempdir(),
            Some(root) => tempfile::Builder::new()
                .prefix("non-utf8-")
                .tempdir_in(root),
        };
        let parent = match parent {
            Ok(parent) => parent,
            Err(error) => {
                refusals.push(format!(
                    "{}: {error}",
                    root.unwrap_or(Path::new("<temporary directory>")).display()
                ));
                continue;
            }
        };
        let path = parent.path().join(&name);
        match std::fs::create_dir(&path) {
            Ok(()) => return Some((parent, path)),
            Err(error) => refusals.push(format!("{}: {error}", parent.path().display())),
        }
    }
    eprintln!(
        "skipping: no directory accepts a non-UTF-8 directory name: {}",
        refusals.join("; ")
    );
    None
}

/// Entry names of `directory`, to check that nothing new was left behind.
#[cfg(unix)]
pub fn entries(directory: &Path) -> Vec<std::ffi::OsString> {
    let mut names = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    names.sort();
    names
}
