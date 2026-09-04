//! `agent` config block defaults (`Config::agent`).
//!
//! Extracted from `config.rs` so the main module stays under the line gate.
//! Pure serde structs + default fns; no behavior lives here beyond
//! serialization defaults.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Serde default for [`AgentDefaults::default`], kept in sync with the
/// `Default` impl so deserializing `{}` yields `"act"` rather than `""`.
/// (Returns `String` to match the field type for `#[serde(default = ...)]`.)
fn default_agent_name() -> String {
    "act".to_string()
}

/// The `agent` config block: default agent name plus file-based-agent
/// resolution knobs (agents root, tool scope, NFS exposure). Every new
/// field defaults, so partial blocks from older configs keep parsing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentDefaults {
    #[serde(default = "default_agent_name")]
    pub default: String,
    /// File-based agents root override. `None` (the default) keeps the
    /// standard resolution (`OPENCODER_AGENTS_DIR` env var, then
    /// `~/.opencoder/agents`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agents_dir: Option<PathBuf>,
    /// Share-tree root (`<share>/todo`, `<share>/env`, `<share>/agent/tools`)
    /// — an NFS-compatible pure-directory layout. `None` (the default) keeps
    /// the standard resolution (`OPENCODER_SHARE_DIR` env var, then
    /// `~/.opencoder/share`); point it at an NFS mount to share templates,
    /// envs and tools across machines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub share_dir: Option<PathBuf>,
    /// Which tool surface custom agents consider: only the active agent's
    /// (`active`, the default) or every registered tool (`all`).
    #[serde(default = "default_tools_scope")]
    pub tools_scope: ToolsScope,
    /// NFS mount used to expose agent workspaces. Disabled by default.
    #[serde(default)]
    pub nfs: AgentNfsConfig,
}
impl Default for AgentDefaults {
    fn default() -> Self {
        AgentDefaults {
            default: "act".to_string(),
            agents_dir: None,
            share_dir: None,
            tools_scope: ToolsScope::Active,
            nfs: AgentNfsConfig::default(),
        }
    }
}

/// Tool-surface breadth for custom agents. Serialized lowercase
/// (`"active"` / `"all"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolsScope {
    Active,
    All,
}

/// Serde default for [`AgentDefaults::tools_scope`] — `active`, in sync
/// with the `Default` impl.
fn default_tools_scope() -> ToolsScope {
    ToolsScope::Active
}

/// NFS exposure of agent workspaces. Disabled by default; `{}`
/// deserializes to loopback-only, read-only, port 2049.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentNfsConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_nfs_host")]
    pub host: String,
    #[serde(default = "default_nfs_port")]
    pub port: u16,
    #[serde(default = "default_true")]
    pub read_only: bool,
}
impl Default for AgentNfsConfig {
    fn default() -> Self {
        AgentNfsConfig {
            enabled: false,
            host: "127.0.0.1".to_string(),
            port: 2049,
            read_only: true,
        }
    }
}

fn default_nfs_host() -> String {
    "127.0.0.1".to_string()
}

fn default_nfs_port() -> u16 {
    2049
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `{}` must deserialize to the `Default` impl for every new field —
    /// partial configs from older binaries keep parsing (serde-default
    /// contract pinned by the existing `agent` block test in
    /// `config/tests.rs`).
    #[test]
    fn agent_defaults_empty_object_matches_default_impl() {
        let ad: AgentDefaults = serde_json::from_str("{}").unwrap();
        let d = AgentDefaults::default();
        assert_eq!(ad.default, d.default);
        assert_eq!(ad.default, "act");
        assert_eq!(ad.agents_dir, None);
        assert_eq!(ad.share_dir, None);
        assert_eq!(ad.tools_scope, d.tools_scope);
        assert_eq!(ad.tools_scope, ToolsScope::Active);
        assert_eq!(ad.nfs, d.nfs);
        assert!(!ad.nfs.enabled);
        assert_eq!(ad.nfs.host, "127.0.0.1");
        assert_eq!(ad.nfs.port, 2049);
        assert!(ad.nfs.read_only);
    }

    /// Explicit values roundtrip through serde with the documented casing,
    /// and unknown keys are tolerated (no `deny_unknown_fields`).
    #[test]
    fn agent_defaults_explicit_values_roundtrip() {
        let raw = r#"{
            "default": "plan",
            "agents_dir": "/tmp/agents",
            "share_dir": "/mnt/nfs/share",
            "tools_scope": "all",
            "nfs": { "enabled": true, "port": 3050, "read_only": false }
        }"#;
        let ad: AgentDefaults = serde_json::from_str(raw).unwrap();
        assert_eq!(ad.default, "plan");
        assert_eq!(
            ad.agents_dir.as_deref(),
            Some(std::path::Path::new("/tmp/agents"))
        );
        assert_eq!(
            ad.share_dir.as_deref(),
            Some(std::path::Path::new("/mnt/nfs/share"))
        );
        assert_eq!(ad.tools_scope, ToolsScope::All);
        assert!(ad.nfs.enabled);
        assert_eq!(ad.nfs.host, "127.0.0.1");
        assert_eq!(ad.nfs.port, 3050);
        assert!(!ad.nfs.read_only);
        // Partial nfs object: missing keys take their own defaults.
        let ad2: AgentDefaults = serde_json::from_str(r#"{ "nfs": { "enabled": true } }"#).unwrap();
        assert!(ad2.nfs.enabled);
        assert_eq!(ad2.nfs.port, 2049);
        let ad3: AgentDefaults = serde_json::from_str(r#"{ "future_key": 1 }"#).unwrap();
        assert_eq!(ad3, AgentDefaults::default());
    }
}
