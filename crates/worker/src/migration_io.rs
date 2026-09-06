use anyhow::{bail, Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::Write,
    path::{Path, PathBuf},
};

pub(crate) fn reject_symlink(path: &Path, label: &str) -> Result<()> {
    crate::layout::reject_symlink(path, label)
}

pub(crate) fn open_lock_file(path: &Path) -> Result<std::fs::File> {
    reject_symlink(path, "node lock")?;
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    options
        .open(path)
        .with_context(|| format!("open node lock {}", path.display()))
}

pub(crate) fn copy_tree_verified(source: &Path, destination: &Path) -> Result<()> {
    if std::fs::symlink_metadata(source)?.file_type().is_symlink() {
        bail!("migration refuses symlink: {}", source.display());
    }
    opencoder_core::share_fs::durable_create_dir_all(destination)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let from = entry.path();
        let to = destination.join(entry.file_name());
        let file_type = entry.file_type()?;
        if to.exists() {
            bail!("migration source trees collide at {}", to.display());
        }
        if file_type.is_symlink() {
            bail!("migration refuses symlink: {}", from.display());
        } else if file_type.is_dir() {
            copy_tree_verified(&from, &to)?;
        } else if file_type.is_file() {
            std::fs::copy(&from, &to)?;
            std::fs::File::open(&to)?.sync_all()?;
            if sha256_file(&from)? != sha256_file(&to)? {
                bail!("migration hash verification failed for {}", from.display());
            }
        }
    }
    std::fs::File::open(destination)?.sync_all()?;
    Ok(())
}

pub(crate) fn trees_equal(left: &Path, right: &Path) -> Result<bool> {
    if !real_directory(left, "migration source")? || !real_directory(right, "migration target")? {
        return Ok(false);
    }
    let mut left_entries = BTreeMap::new();
    for entry in std::fs::read_dir(left)? {
        let entry = entry?;
        left_entries.insert(entry.file_name(), entry);
    }
    let mut right_entries = BTreeMap::new();
    for entry in std::fs::read_dir(right)? {
        let entry = entry?;
        right_entries.insert(entry.file_name(), entry);
    }
    if left_entries.keys().ne(right_entries.keys()) {
        return Ok(false);
    }
    for (name, left_entry) in left_entries {
        if !entries_equal(&left_entry.path(), &right_entries[&name].path())? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn real_directory(path: &Path, label: &str) -> Result<bool> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() {
        bail!("{label} cannot be a symlink: {}", path.display());
    }
    Ok(metadata.is_dir())
}

pub(crate) fn entries_equal(left: &Path, right: &Path) -> Result<bool> {
    let left_type = std::fs::symlink_metadata(left)?.file_type();
    let right_type = match std::fs::symlink_metadata(right) {
        Ok(metadata) => metadata.file_type(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if left_type.is_symlink()
        || right_type.is_symlink()
        || left_type.is_dir() != right_type.is_dir()
        || left_type.is_file() != right_type.is_file()
    {
        return Ok(false);
    }
    if left_type.is_dir() {
        trees_equal(left, right)
    } else {
        Ok(sha256_file(left)? == sha256_file(right)?)
    }
}

pub(crate) fn sha256_file(path: &Path) -> Result<String> {
    let hash = Sha256::digest(std::fs::read(path)?);
    Ok(hash.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub(crate) fn durable_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path.parent().context("migration file has no parent")?;
    opencoder_core::share_fs::durable_create_dir_all(parent)?;
    let temp = parent.join(format!(".migration.tmp-{}", ulid::Ulid::new()));
    let mut file = std::fs::File::create(&temp)?;
    file.write_all(&serde_json::to_vec(value)?)?;
    file.sync_all()?;
    std::fs::rename(&temp, path)?;
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}

pub(crate) fn sync_tree(path: &Path) -> Result<()> {
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            sync_tree(&entry.path())?;
        }
    }
    std::fs::File::open(path)?.sync_all()?;
    Ok(())
}

pub(crate) fn absolute_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

pub(crate) struct Staging(pub(crate) PathBuf);
impl Drop for Staging {
    fn drop(&mut self) {
        if self.0.exists() {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
