//! Ontology storage roots and the independent read-only NFS export.
use crate::{nfs_exports, AppState};
use anyhow::{ensure, Result};
use axum::{
    extract::State,
    response::{IntoResponse, Response},
    Json,
};
use opencoder_core::Config;
use serde::Deserialize;
use serde_json::json;
use std::{
    path::{Component, Path, PathBuf},
    sync::Arc,
};

const EXPORT: &str = "ontology";

pub(crate) fn resolved(path: &Path) -> Result<PathBuf> {
    ensure!(
        path.is_absolute() && !path.components().any(|c| matches!(c, Component::ParentDir)),
        "ontology files must use an absolute path without parent traversal"
    );
    if path.exists() {
        return Ok(path.canonicalize()?);
    }
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("invalid ontology root"))?;
    Ok(resolved(parent)?.join(
        path.file_name()
            .ok_or_else(|| anyhow::anyhow!("invalid ontology root"))?,
    ))
}

pub(crate) fn files_root(config: &Config, workdir: &Path) -> Result<PathBuf> {
    let root = resolved(&config.ontology.files_for(workdir))?;
    let agents = opencoder_agents::default_opts_from_config(config).export_root;
    let binary = config
        .dag
        .binary_dir
        .clone()
        .unwrap_or_else(|| opencoder_core::data_dir_for(workdir).join("dag/binary"));
    for resource in [Some(agents), Some(binary), config.dag.workspace_dir.clone()]
        .into_iter()
        .flatten()
    {
        let resource = resolved(&resource)?;
        ensure!(
            !root.starts_with(&resource) && !resource.starts_with(&root),
            "ontology files cannot overlap an Agent, binary or DAG workspace root"
        );
    }
    Ok(root)
}

pub(crate) fn checked_files_root(config: &Config, workdir: &Path, data: &Path) -> Result<PathBuf> {
    let root = files_root(config, workdir)?;
    ensure!(
        !resolved(data)?.starts_with(&root),
        "ontology export cannot contain Server databases"
    );
    Ok(root)
}

pub(crate) async fn start(config: &Config, workdir: &Path, data: &Path) -> Result<()> {
    if config.ontology.nfs.port != 0 {
        for (enabled, port) in [
            (config.agent.nfs.enabled, config.agent.nfs.port),
            (config.dag.nfs.enabled, config.dag.nfs.port),
            (
                config.dag.workspace_nfs.enabled,
                config.dag.workspace_nfs.port,
            ),
        ] {
            ensure!(
                !enabled || port != config.ontology.nfs.port,
                "ontology NFS port must differ from other exports"
            );
        }
    }
    let root = checked_files_root(config, workdir, data)?;
    tokio::fs::create_dir_all(&root).await?;
    nfs_exports::start(
        EXPORT,
        opencoder_agents::NfsServerOpts {
            export_root: root,
            host: config.ontology.nfs.host.clone(),
            port: config.ontology.nfs.port,
            read_only: true,
        },
    )
    .await
    .map_err(anyhow::Error::msg)?;
    Ok(())
}

pub(crate) async fn autostart(config: &Config, workdir: &Path, data: &Path) -> Result<()> {
    if config.ontology.nfs.enabled {
        start(config, workdir, data).await?;
    }
    Ok(())
}

pub(crate) async fn status(State(state): State<Arc<AppState>>) -> Response {
    match Config::load(&state.workdir)
        .map_err(anyhow::Error::from)
        .and_then(|config| checked_files_root(&config, &state.workdir, &state.data_dir))
    {
        Ok(root) => Json(json!({"ok":true,"root":root,"status":nfs_exports::status(EXPORT).await}))
            .into_response(),
        Err(error) => crate::api::error_500(error.to_string()),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SetBody {
    enabled: bool,
}

pub(crate) async fn set_status(
    State(state): State<Arc<AppState>>,
    Json(body): Json<SetBody>,
) -> Response {
    if body.enabled {
        let result = async {
            let config = Config::load(&state.workdir)?;
            start(&config, &state.workdir, &state.data_dir).await
        }
        .await;
        if let Err(error) = result {
            return crate::api::error_500(error.to_string());
        }
    } else {
        nfs_exports::stop(EXPORT).await;
    }
    status(State(state)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ontology_root_rejects_workspace_overlap_and_symlink_aliases() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("source");
        std::fs::create_dir(&workspace).unwrap();
        let mut config = Config::default();
        config.dag.workspace_dir = Some(workspace.clone());
        config.ontology.files_dir = Some(workspace.join("ontology"));
        assert!(files_root(&config, dir.path()).is_err());
        config.ontology.files_dir = Some(dir.path().join("files"));
        assert_eq!(
            files_root(&config, dir.path()).unwrap(),
            dir.path().join("files")
        );
        #[cfg(unix)]
        {
            let alias = dir.path().join("alias");
            std::os::unix::fs::symlink(&workspace, &alias).unwrap();
            config.ontology.files_dir = Some(alias.join("files"));
            assert!(files_root(&config, dir.path()).is_err());
        }
    }

    #[test]
    fn ontology_export_cannot_contain_server_databases() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        let data = dir.path().join("server");
        config.ontology.files_dir = Some(dir.path().into());
        assert!(checked_files_root(&config, dir.path(), &data).is_err());
        config.ontology.files_dir = Some(data.clone());
        assert!(checked_files_root(&config, dir.path(), &data).is_err());
        config.ontology.files_dir = Some(dir.path().join("files"));
        assert!(checked_files_root(&config, dir.path(), &data).is_ok());
    }
}
