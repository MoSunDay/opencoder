//! Project TODO file storage, separate from workflow execution data.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProjectConfig {
    pub todo_storage: TodoStorageConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TodoStorageConfig {
    pub files_dir: Option<PathBuf>,
    pub max_file_bytes: u64,
    pub uploads_enabled: bool,
    pub nfs_service_url: Option<String>,
    pub nfs: TodoNfsConfig,
}
impl Default for TodoStorageConfig {
    fn default() -> Self {
        Self {
            files_dir: None,
            max_file_bytes: 512 * 1024 * 1024,
            uploads_enabled: true,
            nfs_service_url: None,
            nfs: TodoNfsConfig::default(),
        }
    }
}
impl TodoStorageConfig {
    pub fn files_for(&self, workdir: &Path) -> PathBuf {
        self.files_dir
            .clone()
            .unwrap_or_else(|| crate::data_dir_for(workdir).join("project/todo-files"))
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TodoNfsConfig {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
}
impl Default for TodoNfsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            host: "127.0.0.1".into(),
            port: 2053,
        }
    }
}

pub(super) fn merge(base: &mut serde_json::Value, patch: &serde_json::Value) {
    if let (Some(target), Some(source)) = (base.as_object_mut(), patch.as_object()) {
        for (key, value) in source {
            merge(
                target.entry(key.clone()).or_insert(serde_json::Value::Null),
                value,
            );
        }
    } else {
        *base = patch.clone();
    }
}

pub(super) fn validate(root: &serde_json::Value) -> crate::error::Result<()> {
    if let Some(value) = root.get("project") {
        let parsed: ProjectConfig = serde_json::from_value(value.clone())
            .map_err(|error| crate::error::CoreError::Config(format!("project: {error}")))?;
        if parsed.todo_storage.max_file_bytes == 0 {
            return Err(crate::error::CoreError::Config(
                "project.todo_storage.max_file_bytes must be positive".into(),
            ));
        }
        if let Some(url) = &parsed.todo_storage.nfs_service_url {
            let valid = url
                .strip_prefix("http://127.0.0.1:")
                .and_then(|port| port.parse::<u16>().ok())
                .is_some_and(|port| port > 0);
            if !valid {
                return Err(crate::error::CoreError::Config(
                    "project.todo_storage.nfs_service_url must be loopback HTTP with a port".into(),
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_storage_configuration_is_not_silently_ignored() {
        for patch in [
            serde_json::json!({"project":{"todo_storage":{"nfs":{"read_only":false}}}}),
            serde_json::json!({"project":{"todo_storage":{"max_file_bytes":0}}}),
            serde_json::json!({"project":{"todo_storage":{"nfs_service_url":"http://remote:18085"}}}),
            serde_json::json!({"project":{"todo_storage":{"nfs_service_url":"http://127.0.0.1:0"}}}),
        ] {
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join("opencoder.json"), patch.to_string()).unwrap();
            assert!(crate::Config::load_with_home_frozen(dir.path(), Some(dir.path())).is_err());
        }
    }
    #[test]
    fn defaults_and_nested_overlays_preserve_storage() {
        let config = TodoStorageConfig::default();
        assert_eq!(config.nfs.port, 2053);
        assert!(!config.nfs.enabled);
        assert_eq!(config.max_file_bytes, 512 * 1024 * 1024);
        assert_eq!(
            config.files_for(Path::new("/work")),
            crate::data_dir_for(Path::new("/work")).join("project/todo-files")
        );
        let mut value = serde_json::to_value(ProjectConfig::default()).unwrap();
        merge(
            &mut value,
            &serde_json::json!({"todo_storage":{"files_dir":"/files","nfs":{"enabled":true}}}),
        );
        merge(
            &mut value,
            &serde_json::json!({"todo_storage":{"nfs":{"port":2080}}}),
        );
        let merged: ProjectConfig = serde_json::from_value(value).unwrap();
        assert_eq!(merged.todo_storage.files_dir.unwrap(), Path::new("/files"));
        assert!(merged.todo_storage.nfs.enabled);
        assert_eq!(merged.todo_storage.nfs.port, 2080);
        assert!(serde_json::from_str::<TodoNfsConfig>(r#"{"read_only":false}"#).is_err());
    }
}
