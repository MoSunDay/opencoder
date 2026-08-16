//! Named environment config sets (`~/.opencoder/envs/<name>/`).
//!
//! An env is a directory holding one complete opencoder config snapshot:
//! `config.json` plus the three domain files (`mcp.json` / `cli.json` /
//! `skills.json`). While an env is active — the marker file
//! `<global_opencode_home>/envs/active` holds its name and the env directory
//! exists — config resolution gains an env layer between the project files
//! and the global home:
//!
//! - `config.json`: project > env > `~/.opencoder` > XDG (per-key merge)
//! - domain files: project > env > `~/.opencoder` (first existing file wins)
//!
//! A stale marker (env dir deleted) deactivates silently: [`active_env`]
//! returns `None` and resolution falls back to base behavior. All paths go
//! through [`super::env::global_opencode_home`], so the `scoped_config_home`
//! test override isolates envs too. Pure functions over the filesystem — no
//! process-env reads, no mutable globals.

use std::io;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};

use super::env::global_opencode_home;

/// Marker file under `envs/`: one line, the active env's name.
const ACTIVE_MARKER: &str = "active";

/// Env name length cap (keeps paths and TUI rows sane).
const MAX_NAME_LEN: usize = 48;

/// `~/.opencoder/envs/` — the env root (never created by read-only calls).
pub fn envs_home() -> Option<PathBuf> {
    global_opencode_home().map(|home| home.join("envs"))
}

/// `~/.opencoder/envs/<name>/` for a validated `name`.
pub fn env_dir(name: &str) -> Option<PathBuf> {
    envs_home().map(|root| root.join(name))
}

/// Validate an env name: non-empty, ≤ [`MAX_NAME_LEN`] chars, `[A-Za-z0-9._-]`
/// only, and not `.`/`..` (no path traversal into the marker/env root).
pub fn validate_env_name(name: &str) -> std::result::Result<(), String> {
    if name.is_empty() {
        return Err("名称不能为空".to_string());
    }
    if name.len() > MAX_NAME_LEN {
        return Err(format!("名称过长（>{MAX_NAME_LEN} 字符）"));
    }
    if name == "." || name == ".." {
        return Err("名称不能是 . 或 ..".to_string());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        return Err("只能包含字母、数字、_、-、.".to_string());
    }
    Ok(())
}

/// The active env name, or `None` when no marker exists, the marker is blank,
/// names an invalid/traversal string, or the env directory is gone (stale
/// marker silently deactivates).
pub fn active_env() -> Option<String> {
    let raw = std::fs::read_to_string(envs_home()?.join(ACTIVE_MARKER)).ok()?;
    let name = raw.trim().to_string();
    if name.is_empty() || validate_env_name(&name).is_err() {
        return None;
    }
    match env_dir(&name) {
        Some(dir) if dir.is_dir() => Some(name),
        _ => None,
    }
}

/// Set (`Some`) or clear (`None`) the active-env marker. Setting requires the
/// env to exist; the marker is written last so readers never see a marker
/// pointing at a half-built env.
pub fn set_active_env(name: Option<&str>) -> io::Result<()> {
    let root = envs_home().ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "cannot resolve ~/.opencoder")
    })?;
    match name {
        Some(n) => {
            validate_env_name(n)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
            match env_dir(n) {
                Some(dir) if dir.is_dir() => {}
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("unknown env: {n}"),
                    ))
                }
            }
            std::fs::create_dir_all(&root)?;
            std::fs::write(root.join(ACTIVE_MARKER), format!("{n}\n"))
        }
        None => match std::fs::remove_file(root.join(ACTIVE_MARKER)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        },
    }
}

/// List env names (directories under `envs/`), sorted.
pub fn list_envs() -> Vec<String> {
    let Some(root) = envs_home() else {
        return Vec::new();
    };
    let mut names: Vec<String> = std::fs::read_dir(&root)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();
    names
}

/// Pretty-write `value` to `path` (creating parents). Captured env files may
/// embed provider api keys, so they get owner-only permissions on unix,
/// mirroring [`super::Config::ensure_global_config`].
fn write_private_json(path: &Path, value: &serde_json::Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = format!("{}\n", serde_json::to_string_pretty(value)?);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true).mode(0o600);
        options
            .open(path)
            .with_context(|| format!("write {}", path.display()))?
            .write_all(body.as_bytes())?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, body)?;
    }
    Ok(())
}

// (write_all needs io::Write in scope on unix)
#[cfg(unix)]
use std::io::Write as _;

/// Snapshot the *base* config chain (env layer excluded) into `dir`:
/// the per-key merged `config.json` (domain keys stripped) plus one file per
/// domain that has an effective source; stale env domain files without a
/// source are removed. No env-var overlay is applied — values like
/// `OPENAI_BASE_URL` must not bake into files. The capture includes the
/// project layer (WYSIWYG: the env reproduces what loading would resolve
/// from files alone, `active=None`).
fn capture_into(dir: &Path, working_dir: &Path) -> Result<()> {
    // config.json: file-level per-key merge of the base candidate chain,
    // global-first so project keys win (mirrors `Config::load` order).
    let mut merged = serde_json::json!({});
    let mut candidates = super::env::config_candidates_with(working_dir, None);
    candidates.reverse();
    for p in candidates {
        if let Ok(raw) = std::fs::read_to_string(&p) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                if v.is_object() {
                    super::merge::merge_json(&mut merged, &v);
                }
            }
        }
    }
    if let Some(obj) = merged.as_object_mut() {
        for (key, _) in super::domain::DOMAIN_FILES {
            obj.remove(key);
        }
    }
    if merged.as_object().is_some_and(|o| !o.is_empty()) {
        write_private_json(&dir.join("config.json"), &merged)?;
    }
    // Domain files: snapshot the effective base file (project > global);
    // remove stale env copies that no longer have a source.
    for (key, file) in super::domain::DOMAIN_FILES {
        let target = dir.join(file);
        match super::domain::read_effective_with(working_dir, key, None) {
            Some(v) => write_private_json(&target, &v)?,
            None => {
                let _ = std::fs::remove_file(&target);
            }
        }
    }
    Ok(())
}

/// Create a new env, optionally seeded from a base-chain capture.
/// Fails on invalid or duplicate names. Returns the env directory.
pub fn create_env(name: &str, working_dir: &Path, capture: bool) -> Result<PathBuf> {
    validate_env_name(name).map_err(|e| anyhow!("invalid env name: {e}"))?;
    let dir = env_dir(name).ok_or_else(|| anyhow!("cannot resolve ~/.opencoder"))?;
    if dir.exists() {
        bail!("env already exists: {name}");
    }
    std::fs::create_dir_all(&dir)?;
    if capture {
        capture_into(&dir, working_dir).with_context(|| format!("capture into env {name}"))?;
    }
    Ok(dir)
}

/// Re-capture the base chain into an existing env (full replace).
pub fn recapture_env(name: &str, working_dir: &Path) -> Result<()> {
    validate_env_name(name).map_err(|e| anyhow!("invalid env name: {e}"))?;
    match env_dir(name) {
        Some(dir) if dir.is_dir() => {
            capture_into(&dir, working_dir).with_context(|| format!("recapture env {name}"))
        }
        _ => bail!("unknown env: {name}"),
    }
}

/// Delete an env directory. When it is the active env the marker is cleared
/// *first* (fixed order: `active_env` also tolerates a stale marker, so an
/// interrupted delete never corrupts resolution).
pub fn delete_env(name: &str) -> Result<()> {
    validate_env_name(name).map_err(|e| anyhow!("invalid env name: {e}"))?;
    let dir = env_dir(name).ok_or_else(|| anyhow!("cannot resolve ~/.opencoder"))?;
    if !dir.is_dir() {
        bail!("unknown env: {name}");
    }
    if active_env().as_deref() == Some(name) {
        set_active_env(None)?;
    }
    std::fs::remove_dir_all(&dir)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scoped() -> (tempfile::TempDir, crate::ScopedConfigHome) {
        let home = tempfile::tempdir().unwrap();
        let guard = crate::config::scoped_config_home(home.path().to_path_buf());
        (home, guard)
    }

    #[test]
    fn validate_env_name_accepts_and_rejects() {
        assert!(validate_env_name("work").is_ok());
        assert!(validate_env_name("MyEnv-2.b").is_ok());
        for bad in ["", ".", "..", "a/b", "../x", "a b", "中文", &"x".repeat(49)] {
            assert!(validate_env_name(bad).is_err(), "{bad:?} should be invalid");
        }
    }

    #[test]
    fn marker_roundtrip_and_stale_fallback() {
        let (home, _g) = scoped();
        let root = home.path().join(".opencoder").join("envs");
        std::fs::create_dir_all(root.join("alpha")).unwrap();
        assert!(active_env().is_none(), "no marker yet");
        assert!(list_envs() == vec!["alpha".to_string()]);

        set_active_env(Some("alpha")).unwrap();
        assert_eq!(active_env().as_deref(), Some("alpha"));

        // stale marker: env dir removed without clearing the marker
        std::fs::remove_dir_all(root.join("alpha")).unwrap();
        assert!(active_env().is_none(), "stale marker deactivates silently");

        set_active_env(Some("missing")).unwrap_err();
        set_active_env(Some("../evil")).unwrap_err();
        set_active_env(None).unwrap();
        assert!(active_env().is_none());
    }

    #[test]
    fn create_rejects_duplicates_and_delete_clears_marker_first() {
        let (home, _g) = scoped();
        let work = tempfile::tempdir().unwrap();
        assert!(create_env("beta", work.path(), false).unwrap().exists());
        assert!(create_env("beta", work.path(), false).is_err(), "duplicate");
        assert!(create_env("bad name", work.path(), false).is_err());

        set_active_env(Some("beta")).unwrap();
        delete_env("beta").unwrap();
        assert!(!home.path().join(".opencoder/envs/beta").exists());
        assert!(active_env().is_none(), "marker cleared by delete");
        assert!(delete_env("beta").is_err(), "unknown env");
    }
}
