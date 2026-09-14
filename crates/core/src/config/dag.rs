//! `dag` config block defaults (`Config::dag`).
//!
//! Extracted from `config.rs` following the `agent` block pattern so the
//! main module stays under the line gate. Pure serde structs + default
//! fns; no behavior lives here beyond serialization defaults.

use serde::{Deserialize, Serialize};

/// DAG wasm-module pool + NFS exposure knobs (`Config::dag`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DagConfig {
    /// Wasm module pool root (`<name>/v{n}/wasm.bin` tree). `None` = derive
    /// the default `<data_dir>/dag/wasm` at the serving daemon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wasm_dir: Option<std::path::PathBuf>,
    /// Read-only NFS export of the wasm pool for nodes. Disabled by default.
    #[serde(default)]
    pub nfs: DagNfsConfig,
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
}
