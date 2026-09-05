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
    agent_ref_current_dir(reference, "skills")
        .into_iter()
        .collect()
}

/// Tool dirs for one agent: `current.tools` ref → shared tools pool
/// current version dir (0–1 entries; silent empty).
pub fn agent_tools_dirs(agent_name: &str) -> Vec<PathBuf> {
    let reference = read_agent_meta(agent_name).and_then(|m| m.current.tools);
    agent_ref_current_dir(reference, "tools")
        .into_iter()
        .collect()
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

/// Resolve the tool directories a session should expose (the read path
/// behind `agent.tools_scope`):
///
/// - `All` → current version dirs of **every** tools resource (union surface);
/// - `Active` + explicit agent name → that agent's `current.tools` ref;
/// - `Active` + `None` → the active agent's (empty when no marker).
pub fn tools_paths(scope: crate::config::ToolsScope, agent: Option<&str>) -> Vec<PathBuf> {
    match scope {
        crate::config::ToolsScope::All => all_tools_dirs(),
        crate::config::ToolsScope::Active => match agent {
            Some(name) => agent_tools_dirs(name),
            None => active_tools_dirs(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::meta::tests::OVERRIDE_LOCK;
    use crate::agent::meta::{set_active_agent, set_agents_dir_override, AgentMeta, AgentRefs};
    use crate::config::ToolsScope;

    /// Point the agents root at a fresh tempdir under the shared override
    /// lock (the override is process-global; tests must hold the lock for
    /// their whole body to avoid racing `meta`/`agent::tests` fixtures).
    fn scoped() -> (tempfile::TempDir, std::sync::MutexGuard<'static, ()>) {
        let dir = tempfile::tempdir().unwrap();
        let guard = OVERRIDE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_agents_dir_override(Some(dir.path().to_path_buf()));
        (dir, guard)
    }

    /// `tools/<name>/v{v}/` with one file plus a `meta.json` whose
    /// `current` points at `v` (serde-built from [`ResourceMeta`] so the
    /// on-disk shape can never drift from the reader).
    fn make_tools(root: &std::path::Path, name: &str, v: u32) {
        let res = root.join("tools").join(name);
        let vdir = res.join(format!("v{v}"));
        std::fs::create_dir_all(&vdir).unwrap();
        std::fs::write(vdir.join("run.sh"), b"x").unwrap();
        std::fs::write(
            res.join("meta.json"),
            serde_json::to_string(&ResourceMeta {
                name: name.into(),
                current: v,
                history: vec![v],
                ..Default::default()
            })
            .unwrap(),
        )
        .unwrap();
    }

    /// One agent reference card `<name>/meta.json` whose `current.tools`
    /// is `tools` (None = no tools ref).
    fn make_agent(root: &std::path::Path, name: &str, tools: Option<&str>) {
        std::fs::create_dir_all(root.join(name)).unwrap();
        std::fs::write(
            root.join(name).join("meta.json"),
            serde_json::to_string(&AgentMeta {
                name: name.into(),
                current: AgentRefs {
                    tools: tools.map(Into::into),
                    ..Default::default()
                },
                ..Default::default()
            })
            .unwrap(),
        )
        .unwrap();
    }

    /// Three scopes + no-ref emptiness + `current` bump all in one
    /// fixture: `All` unions every pool's current dir, `Active` follows
    /// the named/active agent's `current.tools` ref, and a bumped
    /// `current` moves the resolved dir (the read side never consults
    /// history).
    #[test]
    fn tools_paths_covers_all_three_scopes() {
        let (tmp, _g) = scoped();
        let root = tmp.path();
        make_tools(root, "a", 1);
        make_tools(root, "b", 1);
        make_agent(root, "worker", Some("b"));
        make_agent(root, "bare", None);
        set_active_agent(Some("worker")).unwrap();

        // All → union of every tools resource's current version dir.
        let all = tools_paths(ToolsScope::All, None);
        assert_eq!(all.len(), 2);
        // Active + explicit name → that agent's tools ref.
        let named = tools_paths(ToolsScope::Active, Some("worker"));
        assert_eq!(named, vec![root.join("tools/b/v1")]);
        // Active + None → the active agent's tools ref.
        assert_eq!(tools_paths(ToolsScope::Active, None), named);
        // An agent with no tools ref resolves to the empty surface.
        assert!(tools_paths(ToolsScope::Active, Some("bare")).is_empty());

        // Bumping the pool's current moves the resolved dir.
        make_tools(root, "b", 2);
        assert_eq!(
            tools_paths(ToolsScope::Active, Some("worker")),
            vec![root.join("tools/b/v2")]
        );
        assert_eq!(read_resource_meta("tools", "b").unwrap().current, 2);

        set_agents_dir_override(None);
    }
}
