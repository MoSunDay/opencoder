mod model;
mod query;
#[cfg(test)]
mod tests;

use crate::{api::resolve_env, error::AppError, http::AppState};
use axum::{
    extract::{Path, RawQuery, State},
    routing::get,
    Json, Router,
};
use model::{observe, GraphResponse};
use query::GraphQuery;

pub(crate) fn routes() -> Router<AppState> {
    Router::new().route("/envs/:env/graph", get(graph))
}

async fn graph(
    State(state): State<AppState>,
    Path(env): Path<String>,
    RawQuery(raw): RawQuery,
) -> Result<Json<GraphResponse>, AppError> {
    let query = GraphQuery::parse(raw.as_deref().unwrap_or(""))?;
    let env = resolve_env(&state, &env).await?;
    let store = state.database.store().await;
    let (nodes, edges) = tokio::try_join!(
        store.all_entities(env.env_num),
        store.relationships(env.env_num, None, false),
    )?;
    Ok(Json(observe(nodes, edges, &query)))
}
