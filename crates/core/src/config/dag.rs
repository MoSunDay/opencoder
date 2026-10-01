use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DagConfig {
    #[serde(skip)]
    pub execution_private_root: Option<std::path::PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary_dir: Option<std::path::PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<std::path::PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_dir: Option<std::path::PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rootfs_dir: Option<std::path::PathBuf>,
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
