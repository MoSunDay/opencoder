//! MCP (Model Context Protocol) server configuration.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Configuration for a single MCP server.
///
/// Supports two transports:
/// - **stdio**: spawn `command` with `args`, injecting `env`.
/// - **SSE**: connect to `url`.
///
/// `enabled` gates whether the server is surfaced to the model (only
/// enabled servers are injected into the system prompt).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpServerConfig {
    /// When `false` the server is configured but not surfaced to the model.
    #[serde(default)]
    pub enabled: bool,
    /// Executable to spawn for stdio transport (e.g. `"npx"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Arguments passed to `command`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// Environment variables injected into the spawned process.
    /// Values may use `{VAR}` indirection resolved via `env::resolve_env`.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,
    /// SSE transport URL (alternative to `command`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Merge a JSON object patch into a single `McpServerConfig` entry, field by
/// field (siblings preserved, mirroring the `providers` merge pattern).
pub(super) fn merge(cfg: &mut McpServerConfig, obj: &serde_json::Map<String, serde_json::Value>) {
    if let Some(b) = obj.get("enabled").and_then(|v| v.as_bool()) {
        cfg.enabled = b;
    }
    if let Some(c) = obj.get("command").and_then(|v| v.as_str()) {
        cfg.command = if c.is_empty() { None } else { Some(c.to_string()) };
    }
    if let Some(u) = obj.get("url").and_then(|v| v.as_str()) {
        cfg.url = if u.is_empty() { None } else { Some(u.to_string()) };
    }
    if let Some(arr) = obj.get("args").and_then(|v| v.as_array()) {
        let mapped: Vec<String> = arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
        if !mapped.is_empty() {
            cfg.args = mapped;
        }
    }
    if let Some(map) = obj.get("env").and_then(|v| v.as_object()) {
        for (k, v) in map {
            if let Some(val) = v.as_str() {
                cfg.env.insert(k.clone(), super::env::resolve_env(val));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_server_config_defaults_disabled() {
        let cfg = McpServerConfig::default();
        assert!(!cfg.enabled);
        assert!(cfg.command.is_none());
        assert!(cfg.url.is_none());
        assert!(cfg.args.is_empty());
        assert!(cfg.env.is_empty());
    }

    #[test]
    fn mcp_server_config_roundtrip_serde() {
        let mut env = HashMap::new();
        env.insert("API_KEY".to_string(), "secret".to_string());
        let cfg = McpServerConfig {
            enabled: true,
            command: Some("npx".to_string()),
            args: vec!["-y".to_string(), "@mcp/server".to_string()],
            env,
            url: None,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let back: McpServerConfig = serde_json::from_str(&json).unwrap();
        assert!(back.enabled);
        assert_eq!(back.command.as_deref(), Some("npx"));
        assert_eq!(back.args, vec!["-y", "@mcp/server"]);
        assert_eq!(back.env.get("API_KEY").map(|s| s.as_str()), Some("secret"));
    }

    #[test]
    fn mcp_server_config_disabled_omits_optional_fields_in_json() {
        let cfg = McpServerConfig::default();
        let json = serde_json::to_string(&cfg).unwrap();
        // Optional fields use skip_serializing_if so default serializes to just {"enabled":false}
        assert_eq!(json, r#"{"enabled":false}"#);
    }

    #[test]
    fn merge_updates_enabled_and_command() {
        let mut cfg = McpServerConfig {
            enabled: false,
            command: Some("old".to_string()),
            ..Default::default()
        };
        let patch = serde_json::json!({ "enabled": true, "command": "new" });
        merge(&mut cfg, patch.as_object().unwrap());
        assert!(cfg.enabled);
        assert_eq!(cfg.command.as_deref(), Some("new"));
    }

    #[test]
    fn merge_preserves_unset_fields() {
        let mut cfg = McpServerConfig {
            enabled: true,
            command: Some("npx".to_string()),
            ..Default::default()
        };
        let patch = serde_json::json!({ "enabled": false });
        merge(&mut cfg, patch.as_object().unwrap());
        // command untouched
        assert_eq!(cfg.command.as_deref(), Some("npx"));
    }

    #[test]
    fn merge_empty_command_clears_field() {
        let mut cfg = McpServerConfig {
            command: Some("npx".to_string()),
            ..Default::default()
        };
        let patch = serde_json::json!({ "command": "" });
        merge(&mut cfg, patch.as_object().unwrap());
        assert!(cfg.command.is_none());
    }

    #[test]
    fn merge_args_replaces_nonempty() {
        let mut cfg = McpServerConfig {
            args: vec!["old".to_string()],
            ..Default::default()
        };
        let patch = serde_json::json!({ "args": ["a", "b"] });
        merge(&mut cfg, patch.as_object().unwrap());
        assert_eq!(cfg.args, vec!["a", "b"]);
    }
}
