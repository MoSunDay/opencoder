//! Atomic write helpers — the only way anything in the agents tree lands
//! on disk. Mirrors `write_marker_atomic` in
//! `opencoder_core::config::envs` (and `agent::meta`): unique temp
//! sibling + write + fsync + rename + best-effort parent-dir fsync,
//! owner-only 0o600 on unix, temp removed on failure.

use std::io;
use std::path::Path;

use serde::Serialize;

/// RFC3339 UTC timestamp — the canonical format for every `created_at` /
/// `updated_at` / history `at` field in agent cards and resource metas.
pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Atomically replace `path` with `bytes`: write + fsync a unique temp
/// sibling, then rename over the target (atomic within one directory on
/// unix). A best-effort parent-directory fsync makes the rename durable
/// too. The temp sibling is removed on failure — an interrupted writer
/// leaves nothing behind. `meta.json` and version files share this path,
/// so readers never see a torn file.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().map(Path::to_path_buf).unwrap_or_else(|| {
        // A bare filename carries no parent component; fall back to "."
        // so the temp sibling still lands next to the target.
        Path::new(".").to_path_buf()
    });
    let temp = parent.join(unique_temp_name(path));
    let write = || -> io::Result<()> {
        #[cfg(unix)]
        let mut file = {
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&temp)?
        };
        #[cfg(not(unix))]
        let mut file = std::fs::File::create(&temp)?;
        io::Write::write_all(&mut file, bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temp, path)?;
        sync_dir_best_effort(&parent);
        Ok(())
    };
    match write() {
        Ok(()) => Ok(()),
        Err(e) => {
            // Never leave the temp sibling behind on failure.
            let _ = std::fs::remove_file(&temp);
            Err(e)
        }
    }
}

/// JSON flavor of [`atomic_write`]: serialize pretty, replace atomically.
/// Every `meta.json` (card or resource) write goes through here.
pub fn atomic_write_json<T: Serialize + ?Sized>(path: &Path, value: &T) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    atomic_write(path, &bytes)
}

/// Best-effort directory fsync (durability of a rename / dir entry).
pub(crate) fn sync_dir_best_effort(dir: &Path) {
    #[cfg(unix)]
    if let Ok(handle) = std::fs::File::open(dir) {
        let _ = handle.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

/// Unique temp sibling name: `<file>.tmp-<pid>-<nanos>` — collision-safe
/// across writers from the same process and across quick retries.
fn unique_temp_name(target: &Path) -> String {
    let stem = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("opencoder.tmp");
    format!("{stem}.tmp-{}-{}", std::process::id(), nanos())
}

fn nanos() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0)
}

/// `InvalidInput` error (bad category / name / rel_path).
pub(crate) fn invalid_input(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, msg.into())
}

/// `NotFound` error (unresolvable agents root, missing card).
pub(crate) fn not_found(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, msg.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_content_and_parses_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meta.json");
        atomic_write(&path, b"one").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"one");
        atomic_write(&path, b"two").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
        // No temp siblings left next to the target.
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(leftovers.is_empty());
        // Owner-only on unix, matching the envs marker writer.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert!(chrono::DateTime::parse_from_rfc3339(&now_rfc3339()).is_ok());
    }

    #[test]
    fn atomic_write_json_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("card.json");
        let value = serde_json::json!({ "name": "work", "current": 2 });
        atomic_write_json(&path, &value).unwrap();
        let back: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(back, value);
    }
}
