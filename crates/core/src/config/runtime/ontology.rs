use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OntologyConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files_dir: Option<PathBuf>,
    #[serde(default)]
    pub nfs: OntologyNfsConfig,
}

impl OntologyConfig {
    pub fn files_for(&self, workdir: &Path) -> PathBuf {
        self.files_dir
            .clone()
            .unwrap_or_else(|| crate::data_dir_for(workdir).join("ontology/files"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OntologyNfsConfig {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
}

impl Default for OntologyNfsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            host: "127.0.0.1".into(),
            port: 2052,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_export_is_independent_and_paths_follow_workdir() {
        let config: OntologyConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(config.nfs.port, 2052);
        assert!(!config.nfs.enabled);
        assert_eq!(
            config.files_for(Path::new("/work")),
            crate::data_dir_for(Path::new("/work")).join("ontology/files")
        );
        let custom: OntologyConfig = serde_json::from_str(
            r#"{"files_dir":"/data/ontology","nfs":{"enabled":true,"port":0}}"#,
        )
        .unwrap();
        assert_eq!(
            custom.files_for(Path::new("/work")),
            Path::new("/data/ontology")
        );
        assert!(custom.nfs.enabled);
        assert_eq!(custom.nfs.host, "127.0.0.1");
        assert!(serde_json::from_str::<OntologyConfig>(r#"{"nfs":{"read_only":false}}"#).is_err());
    }
}
