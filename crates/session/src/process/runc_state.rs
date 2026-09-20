//! Remove only an empty directory left before runc wrote container metadata.
use std::{fs, io, path::Path};

/// Call while cleaning an owned container after its launcher has exited, or
/// during startup recovery of owned bundles. A successful atomic rmdir proves
/// no state, FIFO, or other entry was removed. Populated directories still need
/// runc's normal cleanup; malformed paths and permission errors stay visible.
pub fn remove_empty_runc_state(state: &Path) -> io::Result<bool> {
    let metadata = match fs::symlink_metadata(state) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(true),
        Err(error) => return Err(error),
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "runc state must be a real directory",
        ));
    }
    match fs::remove_dir(state) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::DirectoryNotEmpty => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_creation_state_is_removed_and_missing_state_is_idempotent() {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("container");
        fs::create_dir(&state).unwrap();
        assert!(remove_empty_runc_state(&state).unwrap());
        assert!(!state.exists());
        assert!(remove_empty_runc_state(&state).unwrap());
        assert!(root.path().is_dir());
    }

    #[test]
    fn metadata_and_unknown_entries_require_normal_container_cleanup() {
        let root = tempfile::tempdir().unwrap();
        for name in ["state.json", "exec.fifo", "unknown"] {
            let state = root.path().join(name);
            fs::create_dir(&state).unwrap();
            fs::write(state.join(name), b"preserved").unwrap();
            assert!(!remove_empty_runc_state(&state).unwrap());
            assert_eq!(fs::read(state.join(name)).unwrap(), b"preserved");
        }
        let file = root.path().join("not-a-directory");
        fs::write(&file, b"preserved").unwrap();
        assert!(remove_empty_runc_state(&file).is_err());
        assert_eq!(fs::read(file).unwrap(), b"preserved");
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_rejected_without_touching_the_target() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target");
        fs::create_dir(&target).unwrap();
        let link = root.path().join("state");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(remove_empty_runc_state(&link).is_err());
        assert!(target.is_dir());
        assert!(link.is_symlink());
        fs::remove_dir(&target).unwrap();
        assert!(remove_empty_runc_state(&link).is_err());
        assert!(link.is_symlink());
    }
}
