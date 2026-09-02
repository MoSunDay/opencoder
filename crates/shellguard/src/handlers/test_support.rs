//! Panic-free filesystem fixtures shared by handler unit tests.
//!
//! The crate denies `unwrap`/`expect`/`panic` for all targets (tests
//! included), so tests build their fixtures through these helpers: directory
//! and file creation is asserted instead of unwrapped, keeping fixture
//! failures loud without tripping the lint gate.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Create a unique directory under the OS temp root and return its path.
pub(crate) fn temp_dir(tag: &str) -> PathBuf {
    static SEQ: AtomicUsize = AtomicUsize::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("shellguard-{}-{n}-{tag}", std::process::id()));
    assert!(
        std::fs::create_dir_all(&dir).is_ok(),
        "failed to create {dir:?}"
    );
    dir
}

/// Write `contents` to `dir/name`, asserting success; returns the file path.
pub(crate) fn write_file(dir: &Path, name: &str, contents: &str) -> PathBuf {
    let path = dir.join(name);
    assert!(
        std::fs::write(&path, contents).is_ok(),
        "failed to write {path:?}"
    );
    path
}

/// Best-effort cleanup of a [`temp_dir`] fixture.
pub(crate) fn cleanup_dir(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}
