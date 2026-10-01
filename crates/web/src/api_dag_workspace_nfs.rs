use crate::{
    nfs_exports::{self, DAG_WORKSPACE_EXPORT},
    AppState,
};
use axum::{
    extract::State,
    response::{IntoResponse, Response},
    Json,
};
use opencoder_agents::NfsServerOpts;
use opencoder_core::Config;
use serde::Deserialize;
use serde_json::json;
use std::{path::Path, sync::Arc};

pub async fn start(config: &Config) -> Result<(), String> {
    let root = config
        .dag
        .workspace_dir
        .clone()
        .ok_or("dag.workspace_dir is required for workspace export")?;
    let metadata = std::fs::symlink_metadata(&root).map_err(|error| error.to_string())?;
    if !root.is_absolute() || !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("DAG workspace export must be an absolute real directory".into());
    }
    nfs_exports::start(
        DAG_WORKSPACE_EXPORT,
        NfsServerOpts {
            export_root: root,
            host: config.dag.workspace_nfs.host.clone(),
            port: config.dag.workspace_nfs.port,
            read_only: true,
        },
    )
    .await?;
    Ok(())
}

pub async fn get_status(State(state): State<Arc<AppState>>) -> Response {
    let result = Config::load(&state.workdir).map(|config| {
        json!({"ok":true,
        "root":config.dag.workspace_dir,"status":{}})
    });
    match result {
        Ok(mut value) => {
            value["status"] = json!(nfs_exports::status(DAG_WORKSPACE_EXPORT).await);
            Json(value).into_response()
        }
        Err(error) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok":false,"error":error.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetBody {
    pub enabled: bool,
}

pub async fn set_status(State(state): State<Arc<AppState>>, Json(body): Json<SetBody>) -> Response {
    if body.enabled {
        let config = match Config::load(&state.workdir) {
            Ok(config) => config,
            Err(error) => {
                return (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"ok":false,"error":error.to_string()})),
                )
                    .into_response()
            }
        };
        if let Err(error) = start(&config).await {
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok":false,"error":error})),
            )
                .into_response();
        }
    } else {
        nfs_exports::stop(DAG_WORKSPACE_EXPORT).await;
    }
    get_status(State(state)).await
}

pub async fn autostart(workdir: &Path) -> anyhow::Result<()> {
    let config = Config::load(workdir)?;
    if config.dag.workspace_nfs.enabled {
        start(&config).await.map_err(anyhow::Error::msg)?;
    }
    Ok(())
}
