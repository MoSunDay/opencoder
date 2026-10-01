use fs2::FileExt;
use std::{
    fs::{File, OpenOptions},
    io,
    path::Path,
};

pub(crate) fn exclusive(root: &Path, name: &str) -> io::Result<File> {
    std::fs::create_dir_all(root)?;
    real_directory(root)?;
    let locks = root.join(".locks");
    std::fs::create_dir_all(&locks)?;
    real_directory(&locks)?;
    let path = locks.join(name);
    if let Ok(metadata) = std::fs::symlink_metadata(&path) {
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(invalid("binary lock must be a regular file"));
        }
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    file.lock_exclusive()?;
    let pool = root.join(name);
    if pool.exists() {
        real_directory(&pool)?;
    }
    Ok(file)
}

fn real_directory(path: &Path) -> io::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(invalid("binary pool paths must be real directories"));
    }
    Ok(())
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
