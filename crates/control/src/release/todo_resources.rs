//! Independent TODO NFS owner; no control database or other resource exports.
use crate::project_files::{self, nfs};
use anyhow::Result;
use axum::{extract::State, response::Response, routing::get, Json, Router};
use opencoder_core::Config;
use std::{path::PathBuf, sync::Arc};

#[derive(Clone)]
struct Paths {
    workdir: PathBuf,
    data: PathBuf,
}

pub async fn serve(workdir: PathBuf, data: PathBuf, port: u16, token: String) -> Result<()> {
    let config = Config::load(&workdir)?;
    project_files::initialize(&config, &workdir, &data)?;
    // Bind management first: a duplicate owner must not start another export.
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    if config.project.todo_storage.nfs.enabled {
        nfs::start(&config, &workdir, &data).await?;
    }
    axum::serve(listener, build_app(workdir, data, token)).await?;
    Ok(())
}

pub fn build_app(workdir: PathBuf, data: PathBuf, token: String) -> Router {
    Router::new()
        .route(
            "/api/health",
            get(|| async {
                Json(serde_json::json!({"ok":true,"role":"todo-resources",
                "build":opencoder_core::version::build_info()}))
            }),
        )
        .route("/api/project/todos/nfs", get(status).post(set_status))
        .with_state(Paths { workdir, data })
        .layer(axum::middleware::from_fn_with_state(
            Some(Arc::new(crate::auth_mw::AuthState::seed_only(token))),
            crate::auth_mw::require_bearer,
        ))
}

async fn status(State(paths): State<Paths>) -> Response {
    nfs::read_status(&paths.workdir, &paths.data).await
}

async fn set_status(State(paths): State<Paths>, Json(body): Json<nfs::SetBody>) -> Response {
    nfs::set_enabled(&paths.workdir, &paths.data, body.enabled).await
}
