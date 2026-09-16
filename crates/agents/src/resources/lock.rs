//! A single cross-process lock for each resource root; no guards cross await points.
use super::filesystem::{check_path, root};
use std::{
    fs::{File, OpenOptions},
    io,
};

pub fn write_lock() -> io::Result<File> {
    let root = root()?;
    check_path(&root)?;
    std::fs::create_dir_all(&root)?;
    let path = root.join(".write.lock");
    check_path(&path)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    file.lock()?;
    Ok(file)
}
