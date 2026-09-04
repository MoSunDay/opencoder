//! Share-tree filesystem protocol — an NFS-compatible pure-directory layout
//! under one root (`<share>/`):
//!
//! ```text
//! <share>/todo/<name>/todo.json                 # template metadata
//! <share>/todo/<name>/<version>/context.json    # WorkflowSpec JSON
//! <share>/todo/<name>/<version>/env.json        # {"env": "<env-name>"}
//! <share>/env/<name>/context.json               # env context
//! <share>/agent/tools/<version>/<tool>          # tool CLIs
//! ```
//!
//! The whole tree can live on an NFS mount: every writer uses atomic
//! tmp+rename, every reader tolerates absent files, and no state lives
//! anywhere else. All functions are pure path/value helpers — no classes,
//! no hidden state beyond the test override below.
//!
//! Resolution order for the root (mirrors `agent::meta::agents_dir`):
//! (a) process-global test override, (b) `OPENCODER_SHARE_DIR` env var,
//! (c) `Config::agent.share_dir`, (d) `<global home>/share`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use anyhow::{bail, Context, Result};

/// Wire prefix for tool references stored in env contexts and specs.
pub const AGENT_TOOLS_PREFIX: &str = "/agent/tools/";

/// Process-global override (tests); `None` in production.
static SHARE_DIR_OVERRIDE: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Monotonic suffix so concurrent atomic writes to the same target never
/// share a tmp file name.
static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn lock_override() -> std::sync::MutexGuard<'static, Option<PathBuf>> {
    SHARE_DIR_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Install (or clear, on `None`) the process-global share-root override.
pub fn set_share_dir_override(dir: Option<PathBuf>) {
    *lock_override() = dir;
}

/// The share root, resolved per the module doc. `config` participates below
/// env/override so an operator env var always wins over a config file.
pub fn effective_share_dir(config: Option<&crate::Config>) -> Option<PathBuf> {
    if let Some(dir) = lock_override().clone() {
        return Some(dir);
    }
    if let Some(v) = crate::config::env::env_get("OPENCODER_SHARE_DIR") {
        let t = v.trim();
        if !t.is_empty() {
            return Some(PathBuf::from(t));
        }
    }
    if let Some(cfg) = config {
        if let Some(d) = cfg.agent.share_dir.clone() {
            return Some(d);
        }
    }
    crate::config::env::global_opencode_home().map(|home| home.join("share"))
}

/// Name rule shared by template/env/tool names and versions: non-empty,
/// ≤128 bytes, no path separators, no `.`/`..`, no NUL (traversal-safe —
/// same spirit as the todos domain id rule).
pub fn validate_share_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("名称不能为空".to_string());
    }
    if name.len() > 128 {
        return Err("名称长度不能超过 128".to_string());
    }
    if name == "." || name == ".." {
        return Err("名称不能是 . 或 ..".to_string());
    }
    if name.contains('/') || name.contains('\\') || name.contains('\0') {
        return Err("名称不能包含路径分隔符".to_string());
    }
    Ok(())
}

fn validate_parts(parts: &[(&str, &str)]) -> Result<()> {
    for (what, part) in parts {
        if let Err(e) = validate_share_name(part) {
            bail!("{what}: {e}");
        }
    }
    Ok(())
}

// ---- path constructors (validated, traversal-safe) ----

pub fn todo_dir(root: &Path, name: &str) -> Result<PathBuf> {
    validate_parts(&[("模板名", name)])?;
    Ok(root.join("todo").join(name))
}

pub fn todo_meta_path(root: &Path, name: &str) -> Result<PathBuf> {
    Ok(todo_dir(root, name)?.join("todo.json"))
}

pub fn todo_version_dir(root: &Path, name: &str, version: &str) -> Result<PathBuf> {
    validate_parts(&[("模板名", name), ("版本", version)])?;
    Ok(root.join("todo").join(name).join(version))
}

pub fn todo_context_path(root: &Path, name: &str, version: &str) -> Result<PathBuf> {
    Ok(todo_version_dir(root, name, version)?.join("context.json"))
}

pub fn todo_env_binding_path(root: &Path, name: &str, version: &str) -> Result<PathBuf> {
    Ok(todo_version_dir(root, name, version)?.join("env.json"))
}

pub fn env_dir(root: &Path, name: &str) -> Result<PathBuf> {
    validate_parts(&[("环境名", name)])?;
    Ok(root.join("env").join(name))
}

pub fn env_context_path(root: &Path, name: &str) -> Result<PathBuf> {
    Ok(env_dir(root, name)?.join("context.json"))
}

pub fn agent_tool_path(root: &Path, version: &str, tool: &str) -> Result<PathBuf> {
    validate_parts(&[("版本", version), ("工具名", tool)])?;
    Ok(root.join("agent").join("tools").join(version).join(tool))
}

// ---- atomic IO ----

/// Atomic write: bytes land in a sibling tmp file, then `rename` swaps it in.
/// Readers on an NFS mount observe either the old or the new file, never a
/// torn one.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_extension(format!("tmp-{}-{}", std::process::id(), n));
    std::fs::write(&tmp, bytes).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

pub fn atomic_write_json(path: &Path, value: &serde_json::Value) -> Result<()> {
    let body = serde_json::to_vec_pretty(value)?;
    atomic_write(path, &body)
}

/// Read + parse a JSON file; `Ok(None)` when it does not exist.
pub fn read_json_opt(path: &Path) -> Result<Option<serde_json::Value>> {
    let raw = match std::fs::read(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("read {}", path.display())),
    };
    Ok(Some(
        serde_json::from_slice(&raw).with_context(|| format!("parse {}", path.display()))?,
    ))
}

/// Sorted subdirectory names of `dir` (missing dir → empty).
pub fn list_child_dirs(dir: &Path) -> Vec<String> {
    list_children(dir, |e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
}

/// Sorted regular-file names of `dir` (missing dir → empty).
pub fn list_child_files(dir: &Path) -> Vec<String> {
    list_children(dir, |e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
}

fn list_children(dir: &Path, keep: impl Fn(&std::fs::DirEntry) -> bool) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| keep(e))
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();
    names
}

// ---- tool references ----

/// Build the canonical ref `/agent/tools/<version>/<tool>`.
pub fn tool_ref(version: &str, tool: &str) -> String {
    format!("{AGENT_TOOLS_PREFIX}{version}/{tool}")
}

/// Resolve `/agent/tools/<version>/<tool>` against a share root. Fails on
/// malformed refs, traversal-shaped parts, or a missing file.
pub fn resolve_tool_ref(root: &Path, reference: &str) -> Result<PathBuf> {
    let rest = reference
        .strip_prefix(AGENT_TOOLS_PREFIX)
        .with_context(|| format!("工具引用必须以 {AGENT_TOOLS_PREFIX} 开头: {reference:?}"))?;
    let mut parts = rest.split('/');
    let version = parts.next().context("工具引用缺少版本段")?;
    let tool = parts.next().context("工具引用缺少工具名段")?;
    if parts.next().is_some() {
        bail!("工具引用段数超限: {reference:?}");
    }
    let path = agent_tool_path(root, version, tool)?;
    if !path.is_file() {
        bail!("工具不存在: {reference}");
    }
    Ok(path)
}
