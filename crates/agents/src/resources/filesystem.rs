//! Strict filesystem reads: missing/corrupt data is an error, never an empty draft.
use super::model::{validate_path, FileEntry, MAX_FILES, MAX_TOTAL_BYTES};
use crate::io::{atomic_write, invalid_input, not_found, sync_dir_best_effort};
use serde::de::DeserializeOwned;
use std::{
    fs, io,
    path::{Path, PathBuf},
};

pub fn root() -> io::Result<PathBuf> {
    opencoder_core::agent::agents_dir().ok_or_else(|| not_found("agent resource root unavailable"))
}

/// Reject symlinks at every component under the configured root, including the root.
pub fn check_path(path: &Path) -> io::Result<()> {
    let root = root()?;
    let relative = path
        .strip_prefix(&root)
        .map_err(|_| invalid_input("path escaped resource root"))?;
    let mut part = root;
    check_component(&part)?;
    for item in relative.components() {
        if !matches!(item, std::path::Component::Normal(_)) {
            return Err(invalid_input("invalid path component"));
        }
        part.push(item);
        check_component(&part)?;
    }
    Ok(())
}
fn check_component(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => Err(invalid_input(format!(
            "symlinks are forbidden: {}",
            path.display()
        ))),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> io::Result<T> {
    check_path(path)?;
    serde_json::from_slice(&fs::read(path)?)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn read_files(dir: &Path) -> io::Result<Vec<FileEntry>> {
    check_path(dir)?;
    let mut files = vec![];
    walk(dir, dir, &mut files, &mut 0, 0)?;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}
fn walk(
    root: &Path,
    dir: &Path,
    files: &mut Vec<FileEntry>,
    total: &mut usize,
    depth: usize,
) -> io::Result<()> {
    if depth > 64 {
        return Err(invalid_input("resource directory too deep"));
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            walk(root, &entry.path(), files, total, depth + 1)?;
        } else if kind.is_file() {
            let metadata = entry.metadata()?;
            if metadata.len() > MAX_TOTAL_BYTES as u64 || files.len() >= MAX_FILES {
                return Err(invalid_input("resource exceeds limits"));
            }
            let bytes = fs::read(entry.path())?;
            *total += bytes.len();
            if *total > MAX_TOTAL_BYTES {
                return Err(invalid_input("resource exceeds 1.5 MiB"));
            }
            let path = entry
                .path()
                .strip_prefix(root)
                .unwrap()
                .to_str()
                .ok_or_else(|| invalid_input("file path is not UTF-8"))?
                .to_string();
            validate_path(&path)?;
            #[cfg(unix)]
            let mode = {
                use std::os::unix::fs::PermissionsExt;
                metadata.permissions().mode() & 0o777
            };
            #[cfg(not(unix))]
            let mode = 0o600;
            files.push(FileEntry { path, bytes, mode });
        } else {
            return Err(invalid_input("resource contains a symlink or special file"));
        }
    }
    Ok(())
}

pub fn write_files(dir: &Path, files: &[FileEntry]) -> io::Result<()> {
    fs::create_dir(dir)?;
    for file in files {
        let target = dir.join(&file.path);
        let parent = target.parent().unwrap();
        fs::create_dir_all(parent)?;
        atomic_write(&target, &file.bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&target, fs::Permissions::from_mode(file.mode))?;
        }
        fs::File::open(&target)?.sync_all()?;
        let mut ancestor = Some(parent);
        while let Some(path) = ancestor.filter(|p| p.starts_with(dir)) {
            sync_dir_best_effort(path);
            ancestor = path.parent();
        }
    }
    sync_dir_best_effort(dir);
    Ok(())
}
