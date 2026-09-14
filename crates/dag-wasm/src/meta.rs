//! Pool metas, root resolution and pure path helpers — the read side of
//! the wasm pool, mirroring `opencode_core::agent::meta` /
//! `agent::resource`. Every read degrades silently (`None` / empty vec):
//! a broken pool must never break its consumers.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::validate::validate_name;

/// Process-global wasm-root override (`Some` wins over the env var and
/// the `None` default). A plain process-wide `Mutex`, read per call so
/// tests and embedders can redirect the root without touching process
/// env — mirrors `AGENTS_OVERRIDE` in `opencode_core::agent::meta`.
static WASM_OVERRIDE: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Read the override slot under its lock (payload cloned out; the guard
/// is dropped before any filesystem work, so reads never serialize on
/// I/O).
fn override_dir() -> Option<PathBuf> {
    let g = WASM_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner());
    g.clone()
}

/// Install (or clear, on `None`) the process-global wasm-root override.
pub fn set_wasm_dir_override(dir: Option<PathBuf>) {
    let mut g = WASM_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner());
    *g = dir;
}

/// The wasm-pool root: (a) task-local scope, (b) process-global
/// override, (c) `OPENCODER_DAG_WASM_DIR` (blank ignored), (d) `None` —
/// callers (web/control middleware) inject the config/data-dir root.
/// Never created.
pub fn wasm_root() -> Option<PathBuf> {
    if let Some(root) = crate::scope::current_root() {
        return Some(root);
    }
    if let Some(dir) = override_dir() {
        return Some(dir);
    }
    if let Ok(v) = std::env::var("OPENCODER_DAG_WASM_DIR") {
        let t = v.trim();
        if !t.is_empty() {
            return Some(PathBuf::from(t));
        }
    }
    None
}

/// `meta.json` for one wasm pool (`<name>/meta.json`). Every field
/// defaults so partial metas keep parsing: a newer writer adding keys
/// must not brick older readers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WasmPoolMeta {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    /// Current version; `0` means "no version yet" (treated as absent).
    #[serde(default)]
    pub current: u32,
    #[serde(default)]
    pub history: Vec<u32>,
}

/// `meta.json` for one version (`<name>/v{n}/meta.json`) — the content
/// digest and size a node pin verifies against. Same all-defaults
/// forward-compat contract as [`WasmPoolMeta`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WasmVersionMeta {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub size_bytes: u64,
    #[serde(default)]
    pub updated_at: String,
}

/// `<root>/<name>` — pure join, no validation, no existence check (the
/// write path validates the name first; a validated name can never
/// traverse).
pub fn pool_dir(root: &Path, name: &str) -> PathBuf {
    root.join(name)
}

/// `<root>/<name>/v{version}` — pure join.
pub fn version_dir(root: &Path, name: &str, version: u32) -> PathBuf {
    pool_dir(root, name).join(format!("v{version}"))
}

/// `<root>/<name>/v{version}/wasm.bin` — the module's fixed binary name.
pub fn wasm_bin(root: &Path, name: &str, version: u32) -> PathBuf {
    version_dir(root, name, version).join("wasm.bin")
}

/// Read and parse `<name>/meta.json`. Any failure (invalid name,
/// missing, unreadable, unparseable) degrades to `None`.
pub fn read_pool_meta(root: &Path, name: &str) -> Option<WasmPoolMeta> {
    validate_name(name).ok()?;
    let raw = std::fs::read_to_string(pool_dir(root, name).join("meta.json")).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Read and parse `<name>/v{version}/meta.json`, same silent-`None`
/// contract as [`read_pool_meta`].
pub fn read_version_meta(root: &Path, name: &str, version: u32) -> Option<WasmVersionMeta> {
    validate_name(name).ok()?;
    let raw = std::fs::read_to_string(version_dir(root, name, version).join("meta.json")).ok()?;
    serde_json::from_str(&raw).ok()
}

/// List pool names under `root`, sorted, mirroring `list_resources` in
/// core resource.rs: directories only, names passing [`validate_name`],
/// and a `meta.json` must be present — a stray directory (temp dirs,
/// hand-dropped junk) can never surface in listings.
pub fn list_pools(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| validate_name(name).is_ok())
        .filter(|name| pool_dir(root, name).join("meta.json").is_file())
        .collect();
    names.sort();
    names
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::scope::with_root_sync;

    /// Serializes tests touching the process-global wasm-root override
    /// (and the env var under it) — mirrors `crates/agents/src/testutil`.
    pub(crate) static OVERRIDE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Empty-object metas parse to all-defaults, and unknown keys from a
    /// newer writer are tolerated on both structs.
    #[test]
    fn metas_default_and_tolerate_unknown_fields() {
        let pool: WasmPoolMeta = serde_json::from_str("{}").unwrap();
        assert_eq!(pool, WasmPoolMeta::default());
        let version: WasmVersionMeta = serde_json::from_str("{}").unwrap();
        assert_eq!(version, WasmVersionMeta::default());
        let pool: WasmPoolMeta =
            serde_json::from_str(r#"{"name":"m","current":3,"future_key":[1]}"#).unwrap();
        assert_eq!(pool.name, "m");
        assert_eq!(pool.current, 3);
        let version: WasmVersionMeta =
            serde_json::from_str(r#"{"version":2,"sha256":"ab","nope":true}"#).unwrap();
        assert_eq!(version.version, 2);
        assert_eq!(version.sha256, "ab");
    }

    /// Scope beats override, override beats env, blank env is ignored,
    /// and the chain bottoms out at `None` (callers must inject a root).
    #[test]
    fn wasm_root_prefers_scope_then_override_then_env() {
        let _g = OVERRIDE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_wasm_dir_override(None);
        std::env::remove_var("OPENCODER_DAG_WASM_DIR");
        assert_eq!(wasm_root(), None);
        std::env::set_var("OPENCODER_DAG_WASM_DIR", "   ");
        assert_eq!(wasm_root(), None); // blank ignored
        std::env::set_var("OPENCODER_DAG_WASM_DIR", "/env-root");
        assert_eq!(wasm_root(), Some("/env-root".into()));
        set_wasm_dir_override(Some("/over".into()));
        assert_eq!(wasm_root(), Some("/over".into()));
        assert_eq!(
            with_root_sync(Some("/scoped".into()), wasm_root),
            Some("/scoped".into())
        );
        // A scope of None just falls through to the lower layers.
        assert_eq!(with_root_sync(None, wasm_root), Some("/over".into()));
        set_wasm_dir_override(None);
        std::env::remove_var("OPENCODER_DAG_WASM_DIR");
    }

    /// Listings surface only valid dirs holding a meta.json, sorted.
    #[test]
    fn list_pools_skips_stray_dirs_and_sorts() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        for name in ["beta", "alpha"] {
            std::fs::create_dir_all(root.join(name)).unwrap();
            std::fs::write(root.join(name).join("meta.json"), "{}").unwrap();
        }
        // No meta.json → invisible.
        std::fs::create_dir_all(root.join("no-meta")).unwrap();
        // Invalid name (space fails the charset) → invisible even with a
        // meta.json.
        std::fs::create_dir_all(root.join("bad name")).unwrap();
        std::fs::write(root.join("bad name").join("meta.json"), "{}").unwrap();
        assert_eq!(list_pools(root), vec!["alpha", "beta"]);
        assert!(list_pools(Path::new("/nonexistent-dag-wasm-root")).is_empty());
    }

    /// Path helpers lay out the documented tree shape.
    #[test]
    fn path_helpers_join_expected_layout() {
        let root = Path::new("/pool");
        assert_eq!(pool_dir(root, "m"), Path::new("/pool/m"));
        assert_eq!(version_dir(root, "m", 3), Path::new("/pool/m/v3"));
        assert_eq!(wasm_bin(root, "m", 3), Path::new("/pool/m/v3/wasm.bin"));
    }
}
