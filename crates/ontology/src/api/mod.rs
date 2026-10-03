mod aspects;
mod definitions;
mod directories;
mod entities;
mod entity_creation;
mod environments;
mod graph;
mod relationships;
mod vectors;

use axum::{routing::get, Router};

use crate::http::{session, AppState};

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/session", get(session))
        .merge(environments::routes())
        .merge(aspects::routes())
        .merge(definitions::routes())
        .merge(directories::routes())
        .merge(entities::routes())
        .merge(graph::routes())
        .merge(relationships::routes())
        .merge(vectors::routes())
}

pub async fn resolve_env(
    state: &AppState,
    key: &str,
) -> Result<crate::domain::Environment, crate::error::AppError> {
    let env = state.database.store().await.environment(key).await?;
    if env.is_deleted {
        return Err(crate::error::AppError::NotFound);
    }
    if env.initialization_status != "ready" {
        return Err(crate::error::AppError::dependency(format!(
            "ENV {} initialization is {}",
            env.env_key, env.initialization_status
        )));
    }
    Ok(env)
}

#[derive(serde::Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub include_deleted: bool,
    #[serde(default)]
    pub offset: u32,
    pub limit: Option<u32>,
}
