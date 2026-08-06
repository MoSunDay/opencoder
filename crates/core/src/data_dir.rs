//! Unified per-workdir data-directory resolution.
//!
//! Every crate that opens a SQLite store (CLI, TUI, Web) must derive the
//! *same* on-disk data dir for the same workdir, otherwise sessions created in
//! one process are invisible to another — surfacing as "session not found" and
//! a detached `opencode[exited]` pane. This module owns the single canonical
//! algorithm so the three former call sites can no longer drift.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

/// Resolve the on-disk data directory for a given workdir.
///
/// The path is `<data_local>/opencoder/<hash>` where `hash` is the hex digest
/// of `DefaultHasher` applied to the *canonical* string form of `workdir`.
///
/// Canonicalizing first means `/proj` and `/proj/` (and symlinks) all collapse
/// to a single data dir. If canonicalization fails (e.g. the directory does
/// not exist yet) the raw path is hashed instead of erroring out. Hashing the
/// string representation — rather than the platform-dependent `Path` `Hash`
/// impl — keeps the mapping stable across runs.
pub fn data_dir_for(workdir: &Path) -> PathBuf {
    let canonical = std::fs::canonicalize(workdir).unwrap_or_else(|_| workdir.to_path_buf());
    let mut h = std::collections::hash_map::DefaultHasher::new();
    canonical.to_string_lossy().hash(&mut h);
    let digest = h.finish();
    let mut base = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."));
    base.push("opencoder");
    base.push(format!("{digest:x}"));
    base
}

#[cfg(test)]
mod tests {
    use super::data_dir_for;
    use std::path::PathBuf;

    #[test]
    fn is_deterministic() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().canonicalize().unwrap();
        assert_eq!(data_dir_for(&real), data_dir_for(&real));
    }

    #[test]
    fn canonicalizes_trailing_slash() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().canonicalize().unwrap();
        // A trailing slash on the input must NOT change the data dir.
        let with_slash: PathBuf = format!("{}/", real.to_string_lossy()).into();
        assert_eq!(
            data_dir_for(&real),
            data_dir_for(&with_slash),
            "trailing slash must collapse to the same data dir"
        );
    }

    #[test]
    fn resolves_symlinks() {
        // Symlinks are a unix feature; skip elsewhere.
        #[cfg(unix)]
        {
            let dir = tempfile::tempdir().unwrap();
            let real = dir.path().canonicalize().unwrap();
            let link_dir = dir.path().join("link-target");
            std::os::unix::fs::symlink(&real, &link_dir).unwrap();
            assert_eq!(
                data_dir_for(&real),
                data_dir_for(&link_dir),
                "symlink and target must map to the same data dir"
            );
        }
    }

    #[test]
    fn distinguishes_different_paths() {
        assert_ne!(
            data_dir_for(std::path::Path::new("/a/b")),
            data_dir_for(std::path::Path::new("/a/bb"))
        );
    }
}
