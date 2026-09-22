//! Private task files are immutable, owner-only and outside public artifact roots.
use super::PrivateExecutionContext;
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::OnceLock,
};

/// Digest the running executable image, including when an upgrade unlinks its pathname.
pub fn runtime_image_digest() -> Result<String, String> {
    static DIGEST: OnceLock<Result<String, String>> = OnceLock::new();
    DIGEST
        .get_or_init(|| {
            let mut file = File::open("/proc/self/exe")
                .map_err(|_| "runtime image unavailable".to_string())?;
            let mut digest = Sha256::new();
            let mut buffer = [0u8; 65536];
            loop {
                let read = file
                    .read(&mut buffer)
                    .map_err(|_| "runtime image read failed".to_string())?;
                if read == 0 {
                    break;
                }
                digest.update(&buffer[..read]);
            }
            Ok(format!("sha256:{:x}", digest.finalize()))
        })
        .clone()
}

pub fn materialize(
    base: &Path,
    execution: &str,
    value: &PrivateExecutionContext,
    now_ms: i64,
) -> Result<PathBuf, String> {
    value.validate(now_ms).map_err(str::to_string)?;
    if !base.is_absolute() || !super::super::valid_id(execution) {
        return Err("invalid private execution path".into());
    }
    let root = base.join("private-executions");
    private_directory(&root)?;
    let path = root.join(execution);
    private_directory(&path)?;
    for (name, content) in &value.files {
        seal(&path.join(name), content.as_bytes())?;
    }
    Ok(path)
}
fn private_directory(path: &Path) -> Result<(), String> {
    for parent in path.ancestors() {
        if let Ok(meta) = std::fs::symlink_metadata(parent) {
            if meta.file_type().is_symlink() {
                return Err("symlink in private task path".into());
            }
        }
    }
    if !path.exists() {
        let result = std::fs::DirBuilder::new().mode(0o700).create(path);
        if result.is_err() && !path.is_dir() {
            return Err("private directory creation failed".into());
        }
    }
    let meta = std::fs::symlink_metadata(path).map_err(|_| "private directory unavailable")?;
    if !meta.is_dir() || meta.permissions().mode() & 0o077 != 0 {
        return Err("private directory must have owner-only access".into());
    }
    Ok(())
}
fn seal(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if path.exists() {
        return verify(path, bytes);
    }
    let parent = path.parent().ok_or("private path has no parent")?;
    let temp = parent.join(format!(".private-{}", ulid::Ulid::new()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)
            .map_err(|_| "private file creation failed")?;
        file.write_all(bytes)
            .map_err(|_| "private file write failed")?;
        file.sync_all().map_err(|_| "private file sync failed")?;
        match std::fs::hard_link(&temp, path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err("private file publication failed".to_string()),
        }
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| "private directory sync failed")?;
        verify(path, bytes)
    })();
    let _ = std::fs::remove_file(temp);
    result
}
fn verify(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(path).map_err(|_| "private file missing")?;
    if !meta.is_file() || meta.permissions().mode() & 0o077 != 0 || meta.len() != bytes.len() as u64
    {
        return Err("private file shape or permission mismatch".into());
    }
    if std::fs::read(path).map_err(|_| "private file readback failed")? != bytes {
        return Err("private task file identity drift".into());
    }
    Ok(())
}
