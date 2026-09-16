//! File-based custom agents (`~/.opencoder/agents/`).
//!
//! The agents root holds four shared, independently versioned resource
//! pools — `prompts/<name>/v{n}/{soul,how,output}.md`,
//! `skills/<name>/v{n}/<skill>/SKILL.md`, `tools/<name>/v{n}/…`,
//! `memory/<name>/v{n}/` (directory-shaped: any safe file tree, the
//! reader aggregates every `*.md`; see [`super::resource`]) — plus one
//! thin reference card per agent: `<agent>/meta.json` naming pool
//! resources by *name* ([`AgentRefs`]). An agent directory holds ONLY its
//! `meta.json`; two agents referencing the same prompt share it, and
//! bumping the pool's `current` version updates both. Read paths degrade
//! silently (blank name, corrupt `meta.json` → `None` / empty lists). The
//! agents root is resolved per call, never created.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::config::env::global_opencoder_home;

// The shared-pool read path lives in `resource.rs`; re-exported here so
// `agent::meta::*` remains the single import surface for the agents root.
pub use super::resource::{
    agent_skill_roots, agent_tools_dirs, all_tools_dirs, category_dir, list_resources,
    read_resource_meta, resource_current_version_dir, resource_version_dir,
    validate_resource_name, ResourceMeta, AGENT_CATEGORIES,
};

/// Agent/resource name length cap (keeps paths and TUI rows sane).
pub(crate) const MAX_NAME_LEN: usize = 48;

/// `meta.json` for one agent — a reference card. Every field defaults so
/// partial metas parse: a newer writer adding keys must not brick older
/// readers. The card references shared pool resources by name
/// ([`AgentRefs`]); it never holds version directories itself.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentMeta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness_profile: Option<String>,
    #[serde(default)]
    pub harness: crate::harness::Harness,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    /// Referenced resource names per category (pool names, not versions).
    #[serde(default)]
    pub current: AgentRefs,
    #[serde(default)]
    pub history: Vec<AgentHistoryEntry>,
    /// Resolved snapshot of what the references point at (write path
    /// fills it; the read path never consults it).
    #[serde(default)]
    pub references: AgentReferences,
}

/// Referenced resource names per category (`None` = category unused).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRefs {
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub skills: Option<String>,
    #[serde(default)]
    pub tools: Option<String>,
    #[serde(default)]
    pub memory: Option<String>,
}

/// One reference change: which `field` moved `from` → `to` resource name
/// (`None` = unset) at time `at`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentHistoryEntry {
    #[serde(default)]
    pub at: String,
    /// One of `prompt` | `skills` | `tools` | `memory`.
    #[serde(default)]
    pub field: String,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
}

/// Resolved snapshot of the referenced content per category; filled by
/// the write path when a card is saved.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentReferences {
    /// Prompt file stems present under the referenced `prompts/<n>/v{n}/`
    /// (`soul`/`how`/`output`).
    #[serde(default)]
    pub prompt_files: Vec<String>,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub tools: Vec<String>,
    /// Whether a memory reference resolves.
    #[serde(default)]
    pub memory: bool,
}

/// Process-global agents-root override (`Some` wins over env var and the
/// `~/.opencoder/agents` default). Mirrors the `DISCOVER_CACHE` static in
/// `skill.rs`: a plain process-wide `Mutex`, read per call so tests and
/// embedders can redirect the root without touching process env.
static AGENTS_OVERRIDE: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Read the override slot under its lock (payload cloned out; the guard is
/// dropped before any filesystem work, so reads never serialize on I/O).
fn override_dir() -> Option<PathBuf> {
    let g = AGENTS_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner());
    g.clone()
}

/// Install (or clear, on `None`) the process-global agents-root override.
pub fn set_agents_dir_override(dir: Option<PathBuf>) {
    let mut g = AGENTS_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner());
    *g = dir;
}

/// The agents root: (a) process-global override, (b) `OPENCODER_AGENTS_DIR`
/// (blank ignored), (c) `<global_opencoder_home()>/agents`. Never created.
pub fn agents_dir() -> Option<PathBuf> {
    if let Some(root) = super::scope::current_root() {
        return Some(root);
    }
    if let Some(dir) = override_dir() {
        return Some(dir);
    }
    if let Ok(v) = std::env::var("OPENCODER_AGENTS_DIR") {
        let t = v.trim();
        if !t.is_empty() {
            return Some(PathBuf::from(t));
        }
    }
    global_opencoder_home().map(|home| home.join("agents"))
}

/// `agents/<name>/` for a validated name (validation first: no traversal
/// paths, and the shared pool names are reserved for non-agent dirs).
pub fn agent_dir(name: &str) -> Option<PathBuf> {
    validate_agent_name(name).ok()?;
    agents_dir().map(|root| root.join(name))
}

/// Name contract: non-empty,
/// ≤48 chars, not `.`/`..`, charset `[A-Za-z0-9._-]`, and none of the
/// reserved shared pool dirs (`prompts`/`skills`/`tools`/`memory`) — so
/// an agent directory can never collide with them.
pub fn validate_agent_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("名称不能为空".to_string());
    }
    if AGENT_CATEGORIES.contains(&name) {
        return Err(format!("名称 {name} 与共享资源池保留名冲突，请换一个名称"));
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

/// Read and parse `<name>/meta.json`. Any failure (invalid name, missing,
/// unreadable, unparseable) degrades to `None` — the envs philosophy: a
/// broken file must never break resolution.
pub fn read_agent_meta(name: &str) -> Option<AgentMeta> {
    let dir = agent_dir(name)?;
    let raw = std::fs::read_to_string(dir.join("meta.json")).ok()?;
    serde_json::from_str(&raw).ok()
}

/// The agent's one-line identity: the first non-empty line of the card's
/// current `prompts/<ref>` pool version `soul.md`. Shared by the web
/// reference-card listing (`GET /api/agents`) and the TUI `/agent` picker;
/// `resolve_file_agent` derives the same description through it so the
/// surfaces cannot drift. `None` = no description (invalid name, missing
/// card, unresolvable prompt reference, or missing/blank `soul.md`) —
/// callers fall back to their own generic label.
pub fn agent_description(name: &str) -> Option<String> {
    let card = read_agent_meta(name)?;
    let prompt_ref = card.current.prompt?;
    let dir = resource_current_version_dir("prompts", &prompt_ref)?;
    let soul = std::fs::read_to_string(dir.join("soul.md")).ok()?;
    soul.lines().map(str::trim).find(|l| !l.is_empty()).map(str::to_string)
}

/// List agent names (directories under the agents root), sorted. The
/// reserved shared pool dirs (`prompts`/`skills`/`tools`/`memory`) are
/// never legal agents; `active` is likewise skipped — 兼容旧安装残留的
/// 激活 marker 文件（全局激活已移除，磁盘残留不能被列为 agent 卡）.
pub fn list_agents() -> Vec<String> {
    let Some(root) = agents_dir() else {
        return Vec::new();
    };
    let mut names: Vec<String> = std::fs::read_dir(&root)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| name != "active" && !AGENT_CATEGORIES.contains(&name.as_str()))
        .collect();
    names.sort();
    names
}

#[cfg(test)]
pub(crate) mod tests;
