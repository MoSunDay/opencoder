//! `dag` config block defaults (`Config::dag`).
//!
//! Extracted from `config.rs` following the `agent` block pattern so the
//! main module stays under the line gate. Pure serde structs + default
//! fns; no behavior lives here beyond serialization defaults.

use serde::{Deserialize, Serialize};

/// DAG wasm-module pool + NFS exposure knobs (`Config::dag`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DagConfig {
    /// Runtime-only owning execution path, never accepted from the public configuration.
    #[serde(skip)]
    pub execution_private_root: Option<std::path::PathBuf>,
    /// Wasm module pool root (`<name>/v{n}/wasm.bin` tree). `None` = derive
    /// the default `<data_dir>/dag/wasm` at the serving daemon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wasm_dir: Option<std::path::PathBuf>,
    /// Read-only NFS export of the wasm pool for nodes. Disabled by default.
    #[serde(default)]
    pub nfs: DagNfsConfig,
    /// Absolute host path exposed to sandboxed steps READ-ONLY at
    /// `/workspace/knowledge` (runc bind mount / in-process preopen).
    /// `None` = no knowledge mount (steps see no `/workspace/knowledge`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge_root: Option<std::path::PathBuf>,
    /// Execution sandbox for `agent` steps. DTO-locked: this is a node
    /// local knob, never part of the DAG wire spec. Default `host` keeps
    /// the historical in-node session runner; `runc` moves the whole
    /// agent session (LLM loop + tools) into a container.
    #[serde(default)]
    pub agent_sandbox: AgentSandbox,
    /// Node-side whitelisted op registry (`dag.ops`): `op_id -> launch
    /// config`. `in_process` wasm steps drive heavy host actions
    /// (deploy scripts, session bring-up, harness runs) through the
    /// `opencoder_run_op` host import; ONLY ops explicitly registered
    /// here can run, and sensitive values reach the child process solely
    /// through `env_keys`-declared node environment variables. Empty by
    /// default (fail-closed: an unregistered op id errors the step).
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub ops: std::collections::BTreeMap<String, DagOpConfig>,
}

/// Launch configuration of ONE registered dag op (`dag.ops.<op_id>`).
///
/// The registry is the node's capability surface: `command` is
/// whitespace-split into argv (no shell), `env_keys` names the ONLY
/// node environment variables passed through to the child, and
/// `timeout_secs` bounds the whole run (kill on expiry).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DagOpConfig {
    /// Launch command: `"<program> [args...]"`, whitespace-split, no
    /// shell interpretation. Absolute paths are the recommended form
    /// (the child gets no `PATH` unless declared in `env_keys`).
    pub command: String,
    /// Node environment variables passed through to the child verbatim
    /// (tokens, cookie paths). Everything else is dropped.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env_keys: Vec<String>,
    /// Wall-clock budget in seconds; the runtime kills the child and
    /// errors the step on expiry. `None` = the runtime default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    /// On cancellation/timeout, signal the controller and allow bounded cleanup
    /// before killing its process group. Zero keeps immediate termination.
    #[serde(default)]
    pub termination_grace_secs: u64,
}

/// Where an `agent` step's session runs. Node-local configuration; the
/// DAG spec protocol stays LOCKED (no sandbox field on agent steps).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentSandbox {
    /// In-node session runner on the host (historical default).
    #[default]
    Host,
    /// Full session runner inside a read-only runc container
    /// (`agent-step-runner` in the prepared rootfs).
    Runc,
}

/// NFS exposure of the DAG wasm pool. Disabled by default; `{}` maps to
/// loopback-only, read-only, port 2050 (2049 is the agents export).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DagNfsConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_dag_nfs_host")]
    pub host: String,
    #[serde(default = "default_dag_nfs_port")]
    pub port: u16,
    #[serde(default = "default_dag_nfs_read_only")]
    pub read_only: bool,
}

impl Default for DagNfsConfig {
    fn default() -> Self {
        DagNfsConfig {
            enabled: false,
            host: "127.0.0.1".to_string(),
            port: 2050,
            read_only: true,
        }
    }
}

/// Serde defaults below are kept in sync with the `Default` impls above so
/// deserializing `{}` (a missing or partial block from an older config)
/// yields exactly `DagConfig::default()`.
fn default_dag_nfs_host() -> String {
    "127.0.0.1".to_string()
}

fn default_dag_nfs_port() -> u16 {
    2050
}

fn default_dag_nfs_read_only() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `{}` must deserialize to the `Default` impl for every field —
    /// partial configs from older binaries keep parsing (serde-default
    /// contract pinned the same way as the `agent` block test).
    #[test]
    fn dag_defaults_empty_object_matches_default_impl() {
        let dc: DagConfig = serde_json::from_str("{}").unwrap();
        let d = DagConfig::default();
        assert_eq!(dc, d);
        assert_eq!(dc.wasm_dir, None);
        assert_eq!(dc.knowledge_root, None);
        assert_eq!(dc.agent_sandbox, AgentSandbox::Host);
        assert_eq!(dc.nfs, d.nfs);
        assert!(!dc.nfs.enabled);
        assert_eq!(dc.nfs.host, "127.0.0.1");
        assert_eq!(dc.nfs.port, 2050);
        assert!(dc.nfs.read_only);

        let nfs: DagNfsConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(nfs, DagNfsConfig::default());
        assert!(!nfs.enabled);
        assert_eq!(nfs.host, "127.0.0.1");
        assert_eq!(nfs.port, 2050);
        assert!(nfs.read_only);
    }

    /// Serializing the default emits `nfs` with all defaults and skips the
    /// unset `wasm_dir`, so round-tripping a default stays byte-stable.
    #[test]
    fn dag_default_serialization_skips_wasm_dir_and_pins_nfs_defaults() {
        let v = serde_json::to_value(DagConfig::default()).unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "agent_sandbox": "host",
                "nfs": {
                    "enabled": false,
                    "host": "127.0.0.1",
                    "port": 2050,
                    "read_only": true
                }
            })
        );
        // And the emitted JSON parses back to the default.
        let back: DagConfig = serde_json::from_value(v).unwrap();
        assert_eq!(back, DagConfig::default());
    }

    /// The knowledge-root + agent-sandbox knobs parse from JSON exactly as
    /// the node config file spells them (absolute path, lowercase mode),
    /// and unknown/omitted values fall back safely.
    #[test]
    fn dag_knowledge_root_and_agent_sandbox_parse() {
        let dc: DagConfig = serde_json::from_str(
            r#"{"knowledge_root": "/home/op/workspace", "agent_sandbox": "runc"}"#,
        )
        .unwrap();
        assert_eq!(
            dc.knowledge_root.as_deref(),
            Some(std::path::Path::new("/home/op/workspace"))
        );
        assert_eq!(dc.agent_sandbox, AgentSandbox::Runc);
        // Round-trip keeps both.
        let back: DagConfig = serde_json::from_value(serde_json::to_value(&dc).unwrap()).unwrap();
        assert_eq!(back, dc);
        // A partial override leaves the other field at its default.
        let only_sandbox: DagConfig = serde_json::from_str(r#"{"agent_sandbox": "runc"}"#).unwrap();
        assert_eq!(only_sandbox.knowledge_root, None);
        assert_eq!(only_sandbox.agent_sandbox, AgentSandbox::Runc);
    }

    /// The ops registry parses from the config-file spelling and keeps
    /// undeclared fields at their defaults; an empty default serializes
    /// to nothing (the pinned default JSON stays byte-stable).
    #[test]
    fn dag_ops_registry_parses_and_defaults_to_empty() {
        let dc: DagConfig = serde_json::from_str(
            r#"{"ops": {"eob_deploy": {
                 "command": "/root/workspace/tools/opencode-review-ops/eob_deploy.sh",
                 "env_keys": ["EOB_TOKEN"],
                 "timeout_secs": 1800
               }}}"#,
        )
        .unwrap();
        let op = &dc.ops["eob_deploy"];
        assert_eq!(
            op.command,
            "/root/workspace/tools/opencode-review-ops/eob_deploy.sh"
        );
        assert_eq!(op.env_keys, vec!["EOB_TOKEN".to_string()]);
        assert_eq!(op.timeout_secs, Some(1800));
        // A bare op falls back to the derived defaults.
        let bare: DagConfig =
            serde_json::from_str(r#"{"ops": {"noop": {"command": "/bin/true"}}}"#).unwrap();
        assert!(bare.ops["noop"].env_keys.is_empty());
        assert_eq!(bare.ops["noop"].timeout_secs, None);
        // Default serializes without an `ops` key and round-trips.
        let v = serde_json::to_value(DagConfig::default()).unwrap();
        assert!(v.get("ops").is_none(), "default must skip ops: {v}");
        let back: DagConfig = serde_json::from_value(v).unwrap();
        assert_eq!(back, DagConfig::default());
        // Round-trip keeps the registered ops byte-stable.
        let back: DagConfig = serde_json::from_value(serde_json::to_value(&dc).unwrap()).unwrap();
        assert_eq!(back, dc);
    }
}
