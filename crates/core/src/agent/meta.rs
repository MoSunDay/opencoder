//! File-based custom agents (`~/.opencoder/agents/`).
//!
//! The agents root holds four shared, independently versioned resource
//! pools — `prompts/<name>/v{n}/{soul,how,output}.md`,
//! `skills/<name>/v{n}/<skill>/SKILL.md`, `tools/<name>/v{n}/…`,
//! `memory/<name>/v{n}/memory.md` (see [`super::resource`]) — plus one
//! thin reference card per agent: `<agent>/meta.json` naming pool
//! resources by *name* ([`AgentRefs`]). An agent directory holds ONLY its
//! `meta.json`; two agents referencing the same prompt share it, and
//! bumping the pool's `current` version updates both. The active agent is
//! named by the single-line marker `agents/active` — same contract as the
//! envs marker ([`crate::config::envs`]), including atomic writes and a
//! preflight-checked variant that rolls the marker back on failure. Read
//! paths degrade silently (stale marker, blank name, corrupt `meta.json`
//! → `None` / empty lists). The agents root is resolved per call, never
//! created.

use std::io;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::config::env::global_opencode_home;

// The shared-pool read path lives in `resource.rs`; re-exported here so
// `agent::meta::*` remains the single import surface for the agents root.
pub use super::resource::{
    active_skill_roots, active_tools_dirs, agent_skill_roots, agent_tools_dirs, all_tools_dirs,
    category_dir, list_resources, read_resource_meta, resource_current_version_dir,
    resource_version_dir, validate_resource_name, AGENT_CATEGORIES, ResourceMeta,
};

/// Marker file under the agents root: one line, the active agent's name.
const ACTIVE_MARKER: &str = "active";

/// Agent/resource name length cap (keeps paths and TUI rows sane).
pub(crate) const MAX_NAME_LEN: usize = 48;

/// `meta.json` for one agent — a reference card. Every field defaults so
/// partial metas parse: a newer writer adding keys must not brick older
/// readers. The card references shared pool resources by name
/// ([`AgentRefs`]); it never holds version directories itself.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentMeta {
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
/// (blank ignored), (c) `<global_opencode_home()>/agents`. Never created.
pub fn agents_dir() -> Option<PathBuf> {
    if let Some(dir) = override_dir() {
        return Some(dir);
    }
    if let Ok(v) = std::env::var("OPENCODER_AGENTS_DIR") {
        let t = v.trim();
        if !t.is_empty() {
            return Some(PathBuf::from(t));
        }
    }
    global_opencode_home().map(|home| home.join("agents"))
}

/// `agents/<name>/` for a validated name (validation first: no traversal
/// paths, and the marker/pool names are reserved for non-agent dirs).
pub fn agent_dir(name: &str) -> Option<PathBuf> {
    validate_agent_name(name).ok()?;
    agents_dir().map(|root| root.join(name))
}

/// Same contract as [`crate::config::envs::validate_env_name`]: non-empty,
/// ≤48 chars, not `.`/`..`, charset `[A-Za-z0-9._-]`, and none of the
/// reserved non-agent names — the `active` marker plus the four shared
/// pool dirs (`prompts`/`skills`/`tools`/`memory`) — so an agent
/// directory can never collide with them.
pub fn validate_agent_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("名称不能为空".to_string());
    }
    if name == ACTIVE_MARKER {
        return Err("名称 active 与激活标记保留名冲突，请换一个名称".to_string());
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

/// The active agent name, or `None` when no marker exists, the marker is
/// blank/invalid, or the agent directory is gone (stale marker deactivates
/// silently, mirrors `active_env`).
pub fn active_agent() -> Option<String> {
    let raw = std::fs::read_to_string(agents_dir()?.join(ACTIVE_MARKER)).ok()?;
    let name = raw.trim().to_string();
    if name.is_empty() || validate_agent_name(&name).is_err() {
        return None;
    }
    match agent_dir(&name) {
        Some(dir) if dir.is_dir() => Some(name),
        _ => None,
    }
}

/// Set (`Some`) or clear (`None`) the active-agent marker. Setting requires
/// the agent directory to exist. The marker is written atomically (temp
/// file + fsync + rename + dir fsync, owner-only 0o600 on unix) so an
/// interrupted writer can never leave a torn marker.
pub fn set_active_agent(name: Option<&str>) -> io::Result<()> {
    let root = agents_dir()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "cannot resolve ~/.opencoder"))?;
    match name {
        Some(n) => {
            validate_agent_name(n).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
            match agent_dir(n) {
                Some(dir) if dir.is_dir() => {}
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("unknown agent: {n}"),
                    ))
                }
            }
            std::fs::create_dir_all(&root)?;
            write_marker_atomic(&root, &format!("{n}\n"))
        }
        None => match std::fs::remove_file(root.join(ACTIVE_MARKER)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        },
    }
}

/// Atomically replace the marker (unique temp sibling + rename; best-effort
/// directory fsync makes the rename durable; owner-only 0o600 on unix).
fn write_marker_atomic(root: &std::path::Path, body: &str) -> io::Result<()> {
    let marker = root.join(ACTIVE_MARKER);
    let unique = format!(
        "{ACTIVE_MARKER}.tmp-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    );
    let temp = root.join(unique);
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
        io::Write::write_all(&mut file, body.as_bytes())?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temp, &marker)?;
        #[cfg(unix)]
        if let Ok(dir) = std::fs::File::open(root) {
            let _ = dir.sync_all();
        }
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

/// Set the active agent with a preflight check. After writing the marker
/// to `Some(name)`, run `agents_root_check` (a dry-run meta parse +
/// compose, supplied by the caller); on failure restore the previous marker
/// and surface `InvalidData`. Deactivation (`None`) passes through
/// unchanged. Mirrors `set_active_env_checked`.
pub fn set_active_agent_checked(
    name: Option<&str>,
    agents_root_check: impl FnOnce() -> Result<(), String>,
) -> io::Result<()> {
    let previous = active_agent();
    set_active_agent(name)?;
    let Some(n) = name else {
        return Ok(());
    };
    if let Err(e) = agents_root_check() {
        // Roll back to the pre-activation marker state (ignore secondary
        // errors — the check error is the one that matters).
        let _ = set_active_agent(previous.as_deref());
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("agent `{n}` fails the activation check: {e}"),
        ));
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

/// List agent names (directories under the agents root), sorted. The
/// reserved non-agent names are skipped: the marker (`active`) and the
/// four shared pool dirs (`prompts`/`skills`/`tools`/`memory`) can never
/// be legal agents, so leftovers must not surface in listings.
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
        .filter(|name| name != ACTIVE_MARKER && !AGENT_CATEGORIES.contains(&name.as_str()))
        .collect();
    names.sort();
    names
}

#[cfg(test)]
pub(crate) mod tests;
