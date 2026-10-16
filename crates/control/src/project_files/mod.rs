//! User-uploaded TODO files. Native execution journals remain on their nodes.
pub(crate) mod nfs;
mod storage;
mod transfer;

use crate::AppState;
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Response},
    Json,
};
use opencoder_core::Config;
use serde_json::json;
use std::{path::PathBuf, sync::Arc};

pub(crate) use storage::initialize;
pub(crate) use transfer::{download, remove, upload};

pub(super) fn error(error: anyhow::Error) -> Response {
    let status = if let Some(error) = error.downcast_ref::<HttpError>() {
        error.0
    } else if let Some(error) = error.downcast_ref::<std::io::Error>() {
        match error.kind() {
            std::io::ErrorKind::NotFound => 404,
            std::io::ErrorKind::AlreadyExists => 409,
            _ => 500,
        }
    } else {
        500
    };
    (
        axum::http::StatusCode::from_u16(status).unwrap(),
        Json(json!({"error":error.to_string()})),
    )
        .into_response()
}
#[derive(Debug)]
pub(super) struct HttpError(pub u16, pub &'static str);
impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.1.fmt(f)
    }
}
impl std::error::Error for HttpError {}

pub(super) struct Access {
    pub root: PathBuf,
    pub directory: String,
    pub max_file_bytes: u64,
    pub uploads_enabled: bool,
    _lock: std::fs::File,
    _global: std::fs::File,
}

pub(super) async fn access(state: &AppState, id: &str) -> anyhow::Result<Access> {
    storage::relative(id).map_err(|_| HttpError(400, "invalid TODO id"))?;
    let config = Config::load(&state.workdir)?;
    let root = storage::root(&config, &state.workdir, &state.data_dir)?;
    // Compare the persisted root on every operation, including config reloads.
    storage::initialize(&config, &state.workdir, &state.data_dir)?;
    let global = storage::global_lock(&root).await?;
    let lock = storage::lock(&root, id).await?;
    let todo = state
        .projects
        .get_todo(id)
        .await?
        .ok_or(HttpError(404, "TODO not found"))?;
    Ok(Access {
        directory: storage::directory(&todo)?,
        root,
        max_file_bytes: config.project.todo_storage.max_file_bytes,
        uploads_enabled: config.project.todo_storage.uploads_enabled,
        _lock: lock,
        _global: global,
    })
}

pub(crate) async fn list(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    let result = async {
        let access = access(&state, &id).await?;
        let root = storage::path(&access.root, &access.directory)?;
        Ok::<_, anyhow::Error>(
            json!({"directory":access.directory, "max_file_bytes":access.max_file_bytes,
            "files":storage::entries(&root)?}),
        )
    }
    .await;
    match result {
        Ok(value) => Json(value).into_response(),
        Err(e) => error(e),
    }
}

pub(crate) async fn delete_todo(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let result = async {
        let access = access(&state, &id).await?;
        let directory = storage::path(&access.root, &access.directory)?;
        if !storage::entries(&directory)?.is_empty() {
            return Err(HttpError(409, "TODO 仍有文件，请先删除任务文件").into());
        }
        // Remove only verified empty directories, before deleting the database record.
        storage::remove_empty_tree(&directory)?;
        if !state.projects.delete_todo(&id).await? {
            return Err(HttpError(404, "TODO not found").into());
        }
        Ok::<_, anyhow::Error>(json!({"deleted":true}))
    }
    .await;
    match result {
        Ok(value) => Json(value).into_response(),
        Err(e) => error(e),
    }
}
