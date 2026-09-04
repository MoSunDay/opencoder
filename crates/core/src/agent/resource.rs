//! Shared resource pools under the agents root.
//!
//! Prompt packs, skill-sets, tool-sets and memory banks are shared,
//! independently versioned pools (`prompts/<name>/v{n}/…`,
//! `skills/<name>/v{n}/…`, `tools/<name>/v{n}/…`, `memory/<name>/v{n}/…`);
//! agents reference them by *name* from their reference card
//! ([`super::meta::AgentRefs`]). Two agents referencing the same prompt
//! share one copy — bumping the pool's `current` version updates both.
//! Every read degrades silently (`None` / empty vec): a broken pool must
//! never break agent resolution. The agents root is resolved per call,
//! never created.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::meta::{active_agent, agents_dir, read_agent_meta, MAX_NAME_LEN};

/// Category tokens (used everywhere: pool dirs, helpers, REST later).
pub const AGENT_CATEGORIES: [&str; 4] = ["prompts", "skills", "tools", "memory"];

/// `meta.json` for one shared resource (one per `prompts/<n>`,
/// `skills/<n>`, `tools/<n>`, `memory/<n>`). Every field defaults so
/// partial metas keep parsing: a newer writer adding keys must not brick
/// older readers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceMeta {
    #[serde(default)]
    pub name: String,
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

fn is_category(cat: &str) -> bool {
    AGENT_CATEGORIES.contains(&cat)
}

/// `<agents_root>/<cat>` for a known category token, else `None`.
pub fn category_dir(cat: &str) -> Option<PathBuf> {
    if !is_category(cat) {
        return None;
    }
    agents_dir().map(|root| root.join(cat))
}

/// Validate a resource name under its category dir: same charset/length
/// rules as agent names, but no `active`/category reserved set —
/// resources live under their category dir, so those names cannot
/// collide. Error style mirrors `envs.rs` (Chinese, envs wording).
pub fn validate_resource_name(cat: &str, name: &str) -> Result<(), String> {
    if !is_category(cat) {
        return Err(format!("未知资源类别: {cat}"));
    }
    if name.is_empty() {
        return Err("名称不能为空".to_string());
    }
    if name.len() > MAX_NAME_LEN {
        return Err(format!("名称过长（>{MAX_NAME_LEN} 字符）"));
    }
    if name == "." || name == ".." {
        return Err("名称不能是 . 或 ..".to_string());
    }
    let ok = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.');
    if !ok {
        return Err("只能包含字母、数字、_、-、.".to_string());
    }
    Ok(())
}

/// Read and parse `<cat>/<name>/meta.json`. Any failure (invalid name,
/// missing, unreadable, unparseable) degrades to `None`.
pub fn read_resource_meta(cat: &str, name: &str) -> Option<ResourceMeta> {
    validate_resource_name(cat, name).ok()?;
    let dir = category_dir(cat)?.join(name);
    let raw = std::fs::read_to_string(dir.join("meta.json")).ok()?;
    serde_json::from_str(&raw).ok()
}

/// `<cat>/<name>/v{version}` for a valid (cat, name) — pure path
/// computation, no existence check (write paths target it directly).
pub fn resource_version_dir(cat: &str, name: &str, version: u32) -> Option<PathBuf> {
    validate_resource_name(cat, name).ok()?;
    Some(category_dir(cat)?.join(name).join(format!("v{version}")))
}

/// The resource's current version dir: meta → `current` (`0` ⇒ `None`) →
/// version dir; the dir must exist, else `None`.
pub fn resource_current_version_dir(cat: &str, name: &str) -> Option<PathBuf> {
    let meta = read_resource_meta(cat, name)?;
    if meta.current == 0 {
        return None;
    }
    let dir = resource_version_dir(cat, name, meta.current)?;
    dir.is_dir().then_some(dir)
}

/// List resource names under a category, sorted; unknown category or an
/// unreadable root → silent empty. Names failing validation are skipped
/// (a stray directory can never surface in listings).
pub fn list_resources(cat: &str) -> Vec<String> {
    let Some(dir) = category_dir(cat) else {
        return Vec::new();
    };
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| validate_resource_name(cat, name).is_ok())
        .collect();
    names.sort();
    names
}

/// One agent's `current.<cat>` reference → the pool's current version dir
/// (0–1 entries; a missing/stale/empty ref yields `None`).
fn agent_ref_current_dir(reference: Option<String>, cat: &str) -> Option<PathBuf> {
    resource_current_version_dir(cat, &reference?)
}

/// Skill roots for one agent: `current.skills` ref → shared skills pool
/// current version dir (0–1 entries; silent empty).
pub fn agent_skill_roots(agent_name: &str) -> Vec<PathBuf> {
    let reference = read_agent_meta(agent_name).and_then(|m| m.current.skills);
    agent_ref_current_dir(reference, "skills").into_iter().collect()
}

/// Tool dirs for one agent: `current.tools` ref → shared tools pool
/// current version dir (0–1 entries; silent empty).
pub fn agent_tools_dirs(agent_name: &str) -> Vec<PathBuf> {
    let reference = read_agent_meta(agent_name).and_then(|m| m.current.tools);
    agent_ref_current_dir(reference, "tools").into_iter().collect()
}

/// Skill roots of the active agent (empty when there is none).
pub fn active_skill_roots() -> Vec<PathBuf> {
    active_agent()
        .map(|name| agent_skill_roots(&name))
        .unwrap_or_default()
}

/// Tool dirs of the active agent (empty when there is none).
pub fn active_tools_dirs() -> Vec<PathBuf> {
    active_agent()
        .map(|name| agent_tools_dirs(&name))
        .unwrap_or_default()
}

/// Current version dirs of EVERY tools resource, sorted — the union
/// surface for `ToolsScope::All`.
pub fn all_tools_dirs() -> Vec<PathBuf> {
    list_resources("tools")
        .iter()
        .filter_map(|name| resource_current_version_dir("tools", name))
        .collect()
}
