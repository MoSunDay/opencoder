//! Resource APIs must publish into the same configured root exported by NFS.
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
    match config.agent.agents_dir {
        Some(root) => opencoder_core::agent::scope::with_root(Some(root), next.run(request)).await,
        None => next.run(request).await,
    }
}
