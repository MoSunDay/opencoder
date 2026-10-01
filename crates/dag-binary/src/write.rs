//! Binary pool mutations: save / rollback / delete — modeled directly on
//! the agents resource pool (`opencode-agents` `write.rs` /
//! `rollback.rs`). All writes are atomic (temp sibling + fsync + rename
//! via `opencoder_agents::io`), version numbers are never reused, and
//! rollback is a pointer-only switch.

use std::io;
use std::path::Path;

use sha2::{Digest, Sha256};

use opencoder_agents::{atomic_write, atomic_write_json, now_rfc3339};

use crate::meta::{
    binary_bin, pool_dir, read_pool_meta, version_dir, BinaryPoolMeta, BinaryVersionMeta,
};
use crate::validate::{validate_binary_bytes, validate_name};

/// `InvalidInput` error (bad name / bad module bytes) — mirrors the
/// private helper in `opencode_agents::io`.
fn invalid_input(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, msg.into())
}

/// `NotFound` error (missing pool).
fn not_found(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, msg.into())
}

/// Lowercase hex sha256 — the content digest recorded in every version
/// meta so a node pin can verify its copy of `binary.bin`.
fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn sync_dir(dir: &Path) -> io::Result<()> {
    std::fs::File::open(dir)?.sync_all()
}

/// Default meta for a first-time pool: `current: 0`, empty history.
fn default_pool_meta(name: &str, description: &str) -> BinaryPoolMeta {
    let now = now_rfc3339();
    BinaryPoolMeta {
        name: name.to_string(),
        description: description.to_string(),
        created_at: now.clone(),
        updated_at: now,
        current: 0,
        history: Vec::new(),
    }
}

/// Next version number: `max(history ∪ {current}) + 1` — numbers are
/// never reused, even after a rollback moved `current` backwards.
fn next_version(meta: &BinaryPoolMeta, directory: &Path) -> io::Result<u32> {
    let mut maximum = meta
        .history
        .iter()
        .copied()
        .chain([meta.current])
        .max()
        .unwrap_or(0);
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        if let Some(version) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.strip_prefix('v'))
            .and_then(|digits| digits.parse::<u32>().ok())
        {
            maximum = maximum.max(version);
        }
    }
    maximum
        .checked_add(1)
        .ok_or_else(|| invalid_input("binary version limit reached"))
}

/// Save a new version of a binary module: `binary.bin` + its version meta
/// are written under a `.tmp-v{n}.<pid>` temp dir sibling, then renamed
/// into place as `<name>/v{n}` (atomic dir swap; `AlreadyExists` if the
/// target exists). Finally the pool meta is updated atomically —
/// `current: n`, `history += [n]`, the pool description tracks the
/// latest version, `updated_at` bumps. On any error inside the temp
/// build the temp dir is removed and the pool meta is untouched — no
/// torn state. Saving over an existing pool name is allowed (it just
/// versions); duplicate-create rejection (409) lives in the HTTP layer.
/// Returns the new version number.
pub fn save_binary_version(
    root: &Path,
    name: &str,
    description: &str,
    bytes: &[u8],
) -> io::Result<u32> {
    publish(root, name, description, bytes, false)
}

pub fn create_binary(root: &Path, name: &str, description: &str, bytes: &[u8]) -> io::Result<u32> {
    publish(root, name, description, bytes, true)
}

fn publish(
    root: &Path,
    name: &str,
    description: &str,
    bytes: &[u8],
    create_only: bool,
) -> io::Result<u32> {
    validate_name(name).map_err(invalid_input)?;
    validate_binary_bytes(bytes).map_err(invalid_input)?;
    let _lock = crate::lock::exclusive(root, name)?;
    let dir = pool_dir(root, name);
    if create_only && dir.join("meta.json").exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "binary pool already exists",
        ));
    }
    std::fs::create_dir_all(&dir)?;
    let mut meta = if dir.join("meta.json").exists() {
        read_pool_meta(root, name).ok_or_else(|| invalid_input("invalid binary pool metadata"))?
    } else {
        default_pool_meta(name, description)
    };
    let next = next_version(&meta, &dir)?;
    let dest = version_dir(root, name, next);
    if dest.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("version dir exists: {}", dest.display()),
        ));
    }
    let temp = dir.join(format!(".tmp-v{next}.{}", std::process::id()));
    let build = || -> io::Result<()> {
        std::fs::create_dir_all(&temp)?;
        atomic_write(&temp.join("binary.bin"), bytes)?;
        atomic_write_json(
            &temp.join("meta.json"),
            &BinaryVersionMeta {
                version: next,
                description: description.to_string(),
                sha256: sha256_hex(bytes),
                size_bytes: bytes.len() as u64,
                updated_at: now_rfc3339(),
            },
        )?;
        std::fs::rename(&temp, &dest)
    };
    if let Err(e) = build() {
        let _ = std::fs::remove_dir_all(&temp);
        return Err(e);
    }
    sync_dir(&dir)?;
    meta.current = next;
    meta.history.push(next);
    meta.description = description.to_string();
    meta.updated_at = now_rfc3339();
    atomic_write_json(&dir.join("meta.json"), &meta)?;
    sync_dir(&dir)?;
    Ok(next)
}

/// Switch a pool's `current` back to `version` and bump `updated_at` —
/// a pointer-only switch; version dirs are never deleted by it, so a
/// later save still takes `max(history ∪ {current}) + 1`. The pool must
/// exist (`NotFound`), the version must be in its `history` and the
/// version's `binary.bin` must still be on disk (`InvalidInput` — no
/// guessing at numbers that were never saved, no resurrecting pruned
/// versions). Rolling back to the current version is a no-op `Ok`.
pub fn rollback_binary(root: &Path, name: &str, version: u32) -> io::Result<()> {
    validate_name(name).map_err(invalid_input)?;
    let _lock = crate::lock::exclusive(root, name)?;
    let dir = pool_dir(root, name);
    let Some(mut meta) = read_pool_meta(root, name) else {
        return Err(not_found(format!("unknown binary pool: {name}")));
    };
    if !meta.history.contains(&version) {
        return Err(invalid_input(format!(
            "版本 v{version} 不在 {name} 的历史中"
        )));
    }
    if !binary_bin(root, name, version).is_file() {
        return Err(invalid_input(format!("版本目录缺失: {name}/v{version}")));
    }
    if meta.current == version {
        return Ok(());
    }
    meta.current = version;
    meta.updated_at = now_rfc3339();
    atomic_write_json(&dir.join("meta.json"), &meta)
}

/// Delete a whole pool (`<name>/` recursively) — the only operation that
/// removes version dirs. Best-effort and idempotent: a missing pool is
/// `Ok(())`. An invalid name is still `InvalidInput` — never touch a
/// path that could not have been listed.
pub fn delete_binary(root: &Path, name: &str) -> io::Result<()> {
    validate_name(name).map_err(invalid_input)?;
    let _lock = crate::lock::exclusive(root, name)?;
    match std::fs::remove_dir_all(pool_dir(root, name)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}
