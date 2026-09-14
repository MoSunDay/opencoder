//! Wasm pool mutations: save / rollback / delete — modeled directly on
//! the agents resource pool (`opencode-agents` `write.rs` /
//! `rollback.rs`). All writes are atomic (temp sibling + fsync + rename
//! via `opencoder_agents::io`), version numbers are never reused, and
//! rollback is a pointer-only switch.

use std::io;
use std::path::Path;

use sha2::{Digest, Sha256};

use opencoder_agents::{atomic_write, atomic_write_json, now_rfc3339};

use crate::meta::{pool_dir, read_pool_meta, version_dir, wasm_bin, WasmPoolMeta, WasmVersionMeta};
use crate::validate::{validate_name, validate_wasm_bytes};

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
/// meta so a node pin can verify its copy of `wasm.bin`.
fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Best-effort directory fsync (durability of the version-dir rename) —
/// the agents-crate helper is crate-private, so the same one-liner lives
/// here.
fn sync_dir_best_effort(dir: &Path) {
    #[cfg(unix)]
    if let Ok(handle) = std::fs::File::open(dir) {
        let _ = handle.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

/// Default meta for a first-time pool: `current: 0`, empty history.
fn default_pool_meta(name: &str, description: &str) -> WasmPoolMeta {
    let now = now_rfc3339();
    WasmPoolMeta {
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
fn next_version(meta: &WasmPoolMeta) -> u32 {
    meta.history
        .iter()
        .copied()
        .chain([meta.current])
        .max()
        .unwrap_or(0)
        + 1
}

/// Save a new version of a wasm module: `wasm.bin` + its version meta
/// are written under a `.tmp-v{n}.<pid>` temp dir sibling, then renamed
/// into place as `<name>/v{n}` (atomic dir swap; `AlreadyExists` if the
/// target exists). Finally the pool meta is updated atomically —
/// `current: n`, `history += [n]`, the pool description tracks the
/// latest version, `updated_at` bumps. On any error inside the temp
/// build the temp dir is removed and the pool meta is untouched — no
/// torn state. Saving over an existing pool name is allowed (it just
/// versions); duplicate-create rejection (409) lives in the HTTP layer.
/// Returns the new version number.
pub fn save_wasm_version(
    root: &Path,
    name: &str,
    description: &str,
    bytes: &[u8],
) -> io::Result<u32> {
    validate_name(name).map_err(invalid_input)?;
    validate_wasm_bytes(bytes).map_err(invalid_input)?;
    let dir = pool_dir(root, name);
    std::fs::create_dir_all(&dir)?;
    let mut meta =
        read_pool_meta(root, name).unwrap_or_else(|| default_pool_meta(name, description));
    let next = next_version(&meta);
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
        atomic_write(&temp.join("wasm.bin"), bytes)?;
        atomic_write_json(
            &temp.join("meta.json"),
            &WasmVersionMeta {
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
    sync_dir_best_effort(&dir);
    meta.current = next;
    meta.history.push(next);
    meta.description = description.to_string();
    meta.updated_at = now_rfc3339();
    atomic_write_json(&dir.join("meta.json"), &meta)?;
    Ok(next)
}

/// Switch a pool's `current` back to `version` and bump `updated_at` —
/// a pointer-only switch; version dirs are never deleted by it, so a
/// later save still takes `max(history ∪ {current}) + 1`. The pool must
/// exist (`NotFound`), the version must be in its `history` and the
/// version's `wasm.bin` must still be on disk (`InvalidInput` — no
/// guessing at numbers that were never saved, no resurrecting pruned
/// versions). Rolling back to the current version is a no-op `Ok`.
pub fn rollback_wasm(root: &Path, name: &str, version: u32) -> io::Result<()> {
    validate_name(name).map_err(invalid_input)?;
    let dir = pool_dir(root, name);
    let Some(mut meta) = read_pool_meta(root, name) else {
        return Err(not_found(format!("unknown wasm pool: {name}")));
    };
    if !meta.history.contains(&version) {
        return Err(invalid_input(format!(
            "版本 v{version} 不在 {name} 的历史中"
        )));
    }
    if !wasm_bin(root, name, version).is_file() {
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
pub fn delete_wasm(root: &Path, name: &str) -> io::Result<()> {
    validate_name(name).map_err(invalid_input)?;
    match std::fs::remove_dir_all(pool_dir(root, name)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::{read_pool_meta, read_version_meta};

    /// A minimal valid module: 8-byte header + payload.
    fn module(payload: &[u8]) -> Vec<u8> {
        [b"\0asm\x01\0\0\0".as_slice(), payload].concat()
    }

    /// No `.tmp-` entries left in `dir` (torn-writer detector).
    fn no_tmp_leftovers(dir: &Path) -> bool {
        std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .all(|e| !e.file_name().to_string_lossy().contains(".tmp-"))
    }

    #[test]
    fn save_creates_v1_with_correct_files_and_meta() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let bytes = module(b"(module)");
        assert_eq!(
            save_wasm_version(root, "adder", "first cut", &bytes).unwrap(),
            1
        );
        assert_eq!(
            std::fs::read(version_dir(root, "adder", 1).join("wasm.bin")).unwrap(),
            bytes
        );
        let vm = read_version_meta(root, "adder", 1).unwrap();
        assert_eq!(vm.version, 1);
        assert_eq!(vm.description, "first cut");
        assert_eq!(vm.sha256, sha256_hex(&bytes));
        assert_eq!(vm.size_bytes, bytes.len() as u64);
        let pm = read_pool_meta(root, "adder").unwrap();
        assert_eq!(pm.name, "adder");
        assert_eq!(pm.description, "first cut");
        assert_eq!(pm.current, 1);
        assert_eq!(pm.history, vec![1]);
        assert!(!pm.created_at.is_empty() && !pm.updated_at.is_empty());
        assert!(no_tmp_leftovers(&root.join("adder")));
    }

    #[test]
    fn versions_are_monotonic_across_rollback() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        save_wasm_version(root, "m", "d1", &module(b"one")).unwrap();
        assert_eq!(
            save_wasm_version(root, "m", "d2", &module(b"two")).unwrap(),
            2
        );
        let pm = read_pool_meta(root, "m").unwrap();
        assert_eq!(pm.history, vec![1, 2]);
        assert_eq!(pm.description, "d2"); // pool description tracks latest
        rollback_wasm(root, "m", 1).unwrap();
        let pm = read_pool_meta(root, "m").unwrap();
        assert_eq!(pm.current, 1);
        assert_eq!(pm.history, vec![1, 2]); // history intact
        assert!(version_dir(root, "m", 2).is_dir()); // dirs intact
        rollback_wasm(root, "m", 1).unwrap(); // same-version no-op
        assert_eq!(read_pool_meta(root, "m").unwrap().current, 1);
        // Never reuse: after the rollback the next save is v3, not v2.
        assert_eq!(
            save_wasm_version(root, "m", "d3", &module(b"three")).unwrap(),
            3
        );
        assert_eq!(read_pool_meta(root, "m").unwrap().history, vec![1, 2, 3]);
    }

    #[test]
    fn save_rejects_bad_name_bad_bytes_and_collisions() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        assert_eq!(
            save_wasm_version(root, "../x", "d", &module(b"m"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        // Bad magic (\0max) → InvalidInput.
        assert_eq!(
            save_wasm_version(root, "ok", "d", b"\0max\x01\0\0\0")
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        // Simulate a version-dir collision by pre-creating v1 out-of-band
        // with no meta.json → next computes 1, dest exists.
        std::fs::create_dir_all(root.join("ok/v1")).unwrap();
        assert_eq!(
            save_wasm_version(root, "ok", "d", &module(b"m"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
    }

    #[test]
    fn failed_write_leaves_no_tmp_leftovers() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let dir = root.join("adder");
        // Block the temp build: a directory named `wasm.bin` inside the
        // exact temp path makes the atomic binary write fail (rename of a
        // file over a directory), deterministically and without relying
        // on filesystem permissions.
        std::fs::create_dir_all(
            dir.join(format!(".tmp-v1.{}", std::process::id()))
                .join("wasm.bin"),
        )
        .unwrap();
        assert!(save_wasm_version(root, "adder", "d", &module(b"m")).is_err());
        assert!(no_tmp_leftovers(&dir)); // temp dir removed entirely
        assert!(!dir.join("meta.json").exists()); // pool meta untouched
        assert!(!dir.join("v1").exists()); // nothing published
    }

    #[test]
    fn rollback_rejects_unknown_pool_version_and_pruned_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        save_wasm_version(root, "kit", "d", &module(b"m")).unwrap();
        assert_eq!(
            rollback_wasm(root, "ghost", 1).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        assert_eq!(
            rollback_wasm(root, "kit", 7).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        // In history but the version dir was pruned out-of-band → rejected.
        save_wasm_version(root, "kit", "d2", &module(b"m2")).unwrap();
        std::fs::remove_dir_all(version_dir(root, "kit", 2)).unwrap();
        assert_eq!(
            rollback_wasm(root, "kit", 2).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            rollback_wasm(root, "../x", 1).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn delete_is_idempotent_and_rejects_bad_names() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        save_wasm_version(root, "gone", "d", &module(b"m")).unwrap();
        assert!(root.join("gone").is_dir());
        delete_wasm(root, "gone").unwrap();
        delete_wasm(root, "gone").unwrap(); // second delete is Ok
        assert!(!root.join("gone").exists());
        assert!(delete_wasm(root, "never-there").is_ok());
        assert_eq!(
            delete_wasm(root, "../x").unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }
}
