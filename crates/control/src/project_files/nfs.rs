//! Dedicated read-only export; its lifetime is owned by the resource service.
use super::{error, storage};
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
use std::{path::Path, sync::Arc};
const EXPORT: &str = "project-todos";

pub(crate) async fn start(config: &Config, workdir: &Path, data: &Path) -> Result<()> {
    let nfs = &config.project.todo_storage.nfs;
    for (enabled, port) in [
        (config.agent.nfs.enabled, config.agent.nfs.port),
        (config.dag.nfs.enabled, config.dag.nfs.port),
        (
            config.dag.workspace_nfs.enabled,
            config.dag.workspace_nfs.port,
        ),
        (config.ontology.nfs.enabled, config.ontology.nfs.port),
    ] {
        ensure!(
            nfs.port == 0 || !enabled || nfs.port != port,
            "TODO NFS port must differ from other exports"
        );
    }
    storage::initialize(config, workdir, data)?;
    let root = storage::root(config, workdir, data)?;
    tokio::fs::create_dir_all(&root).await?;
    nfs_exports::start(
        EXPORT,
        opencoder_agents::NfsServerOpts {
            export_root: root,
            host: nfs.host.clone(),
            port: nfs.port,
            read_only: true,
        },
    )
    .await
    .map_err(anyhow::Error::msg)?;
    Ok(())
}

pub(crate) async fn autostart(config: &Config, workdir: &Path, data: &Path) -> Result<()> {
    if config.project.todo_storage.nfs.enabled
        && config.project.todo_storage.nfs_service_url.is_none()
    {
        start(config, workdir, data).await?;
    }
    Ok(())
}

pub(crate) async fn status(State(state): State<Arc<AppState>>) -> Response {
    read_status(&state.workdir, &state.data_dir).await
}

pub(crate) async fn read_status(workdir: &Path, data: &Path) -> Response {
    match Config::load(workdir)
        .map_err(anyhow::Error::from)
        .and_then(|config| storage::root(&config, workdir, data))
    {
        Ok(root) => Json(json!({"ok":true,"root":root,"status":nfs_exports::status(EXPORT).await}))
            .into_response(),
        Err(e) => error(e),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SetBody {
    pub enabled: bool,
}

pub(crate) async fn set_status(
    State(state): State<Arc<AppState>>,
    Json(body): Json<SetBody>,
) -> Response {
    set_enabled(&state.workdir, &state.data_dir, body.enabled).await
}

pub(crate) async fn set_enabled(workdir: &Path, data: &Path, enabled: bool) -> Response {
    if enabled {
        let result = async { start(&Config::load(workdir)?, workdir, data).await }.await;
        if let Err(e) = result {
            return error(e);
        }
    } else {
        nfs_exports::stop(EXPORT).await;
    }
    read_status(workdir, data).await
}
