//! Resource APIs must publish into the same configured root exported by NFS.
//!
//! The binary pool (`/api/dag/binaries*`) has its own scope middleware,
//! [`crate::api_dag_binaries_nfs::configured_dag_binary`] (shared from web via
//! `#[path]`): it resolves the root from `dag.binary_dir` / the data-dir
//! default and injects it into the `opencoder_dag_binary` task-local scope —
//! deliberately NOT duplicated here; `routes::build_app` layers it directly
//! next to [`configured_agents`].
use crate::AppState;
use axum::{extract::State, middleware::Next, response::Response};
use std::sync::Arc;

pub async fn configured_agents(
    State(state): State<Arc<AppState>>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let path = request.uri().path();
    if path != "/api/agents" && !path.starts_with("/api/agents/") {
        return next.run(request).await;
    }
    let config = match opencoder_core::Config::load(&state.workdir) {
        Ok(config) => config,
        Err(error) => return crate::api::error_500(format!("agent resource config: {error:#}")),
    };
    let root = config
        .agent
        .agents_dir
        .or_else(opencoder_core::agent::agents_dir);
    opencoder_core::agent::scope::with_root(root, next.run(request)).await
}
