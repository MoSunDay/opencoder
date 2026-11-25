use serde::{Deserialize, Serialize};

/// Container backend selected for DAG runs on this node.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DagSandbox {
    /// The OCI runtime (`runc`) is required for every run.
    #[default]
    Runc,
    /// Use the native clone3 sandbox when the node supports it, else `runc`.
    Auto,
    /// Require the native clone3 sandbox; no `runc` dependency.
    Clone3,
}

impl DagSandbox {
    /// Sandbox the native test harnesses configure: `DAG_TEST_SANDBOX` takes
    /// the same values as `dag.sandbox` and is `runc` when unset, so every
    /// existing harness keeps its backend unless a run opts in.
    pub fn from_test_env() -> Self {
        let Ok(raw) = std::env::var("DAG_TEST_SANDBOX") else {
            return Self::default();
        };
        serde_json::from_value(serde_json::Value::String(raw))
            .expect("DAG_TEST_SANDBOX must be runc, auto or clone3")
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DagConfig {
    #[serde(default)]
    pub sandbox: DagSandbox,
    #[serde(skip)]
    pub execution_private_root: Option<std::path::PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary_dir: Option<std::path::PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<std::path::PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_dir: Option<std::path::PathBuf>,
    /// Runtime image directory; its `usr/bin` holds the runners when
    /// `runners_dir` is unset. Platform image replacement rewrites this key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rootfs_dir: Option<std::path::PathBuf>,
    /// Host directory holding `dag-runner` and `agent-step-runner`; bound
    /// read-only at `/run/opencoder/bin` inside every DAG sandbox.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runners_dir: Option<std::path::PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge_root: Option<std::path::PathBuf>,
    #[serde(default)]
    pub nfs: DagNfsConfig,
    #[serde(default)]
    pub workspace_nfs: DagWorkspaceNfsConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DagNfsConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_binary_port")]
    pub port: u16,
    #[serde(default = "read_only")]
    pub read_only: bool,
}

impl Default for DagNfsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            host: default_host(),
            port: default_binary_port(),
            read_only: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DagWorkspaceNfsConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_workspace_port")]
    pub port: u16,
}

impl Default for DagWorkspaceNfsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            host: default_host(),
            port: default_workspace_port(),
        }
    }
}

fn default_host() -> String {
    "127.0.0.1".into()
}
fn default_binary_port() -> u16 {
    2050
}
fn default_workspace_port() -> u16 {
    2051
}
fn read_only() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_have_independent_read_only_exports() {
        let config: DagConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(config, DagConfig::default());
        assert_eq!(config.nfs.port, 2050);
        assert_eq!(config.workspace_nfs.port, 2051);
        assert!(config.nfs.read_only);
        assert!(!config.workspace_nfs.enabled);
        assert_eq!(config.sandbox, DagSandbox::Runc);
    }

    #[test]
    fn test_env_backend_defaults_to_runc_and_accepts_config_values() {
        let previous = std::env::var_os("DAG_TEST_SANDBOX");
        for (raw, expected) in [
            (None, DagSandbox::Runc),
            (Some("clone3"), DagSandbox::Clone3),
            (Some("auto"), DagSandbox::Auto),
            (Some("runc"), DagSandbox::Runc),
        ] {
            match &raw {
                Some(value) => std::env::set_var("DAG_TEST_SANDBOX", value),
                None => std::env::remove_var("DAG_TEST_SANDBOX"),
            }
            assert_eq!(DagSandbox::from_test_env(), expected, "{raw:?}");
        }
        match previous {
            Some(value) => std::env::set_var("DAG_TEST_SANDBOX", value),
            None => std::env::remove_var("DAG_TEST_SANDBOX"),
        }
    }

    #[test]
    fn sandbox_modes_roundtrip_and_unknown_values_fail() {
        for (text, expected) in [
            (r#"{"sandbox":"runc"}"#, DagSandbox::Runc),
            (r#"{"sandbox":"auto"}"#, DagSandbox::Auto),
            (r#"{"sandbox":"clone3"}"#, DagSandbox::Clone3),
        ] {
            let config: DagConfig = serde_json::from_str(text).unwrap();
            assert_eq!(config.sandbox, expected, "{text}");
            assert_eq!(
                serde_json::from_value::<DagConfig>(serde_json::to_value(&config).unwrap())
                    .unwrap(),
                config
            );
        }
        assert!(serde_json::from_str::<DagConfig>(r#"{"sandbox":"host"}"#).is_err());
        assert!(serde_json::from_str::<DagConfig>(r#"{"sandbox":""}"#).is_err());
    }

    #[test]
    fn paths_roundtrip_and_unknown_execution_modes_fail() {
        let config: DagConfig = serde_json::from_str(r#"{"workspace_dir":"/mnt/source","data_dir":"/data/runs","rootfs_dir":"/opt/rootfs","binary_dir":"/mnt/binaries"}"#).unwrap();
        assert_eq!(
            config.workspace_dir.as_deref(),
            Some(std::path::Path::new("/mnt/source"))
        );
        assert_eq!(
            serde_json::from_value::<DagConfig>(serde_json::to_value(&config).unwrap()).unwrap(),
            config
        );
        assert!(serde_json::from_str::<DagConfig>(r#"{"sandbox":"host"}"#).is_err());
    }
}
