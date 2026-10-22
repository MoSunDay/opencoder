use crate::AppState;
use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use opencoder_ontology::{AppError, CatalogOperation};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

pub(crate) fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/ontology/envs/:env/capabilities", get(list))
        .route("/api/ontology/envs/:env/capabilities/:id", get(detail))
        .route("/api/ontology/envs/:env/sync", get(status).post(run))
        .route("/api/ontology/envs/:env/sync/check", get(check))
}

#[derive(Deserialize, Default)]
struct Filter {
    kind: Option<String>,
    query: Option<String>,
    #[serde(default)]
    offset: usize,
    limit: Option<usize>,
    #[serde(default)]
    include_deleted: bool,
}
fn response(result: Result<Value, AppError>) -> Response {
    match result {
        Ok(value) => Json(value).into_response(),
        Err(error) => error.into_response(),
    }
}
async fn read(
    state: &AppState,
    env: String,
    operation: CatalogOperation,
) -> Result<Value, AppError> {
    state
        .ontology
        .as_ref()
        .ok_or(AppError::NotFound)?
        .catalog(env, operation)
        .await
}
async fn list(
    State(state): State<Arc<AppState>>,
    Path(env): Path<String>,
    Query(filter): Query<Filter>,
) -> Response {
    let result = async {
        let mut value = read(&state, env, CatalogOperation::List).await?;
        let query = filter.query.unwrap_or_default().to_lowercase();
        let items: Vec<_> = value["items"].as_array_mut().ok_or_else(|| AppError::dependency("invalid catalog"))?
            .drain(..).filter(|row| {
                (filter.include_deleted || row["is_deleted"] != true)
                    && filter.kind.as_ref().is_none_or(|kind| row["capability"]["kind"] == *kind)
                    && ["name","description","source_id"].iter().any(|field|
                        row["capability"][field].as_str().unwrap_or("").to_lowercase().contains(&query))
            }).collect();
        let total = items.len();
        Ok(json!({"items":items.into_iter().skip(filter.offset).take(filter.limit.unwrap_or(100).min(500)).collect::<Vec<_>>(),"total":total}))
    }.await;
    response(result)
}
async fn detail(
    State(state): State<Arc<AppState>>,
    Path((env, id)): Path<(String, String)>,
) -> Response {
    response(read(&state, env, CatalogOperation::Get(id)).await)
}
async fn status(State(state): State<Arc<AppState>>, Path(env): Path<String>) -> Response {
    response(read(&state, env, CatalogOperation::Status).await)
}
async fn check(State(state): State<Arc<AppState>>, Path(env): Path<String>) -> Response {
    response(super::sync::synchronize(&state, env, true).await)
}
async fn run(State(state): State<Arc<AppState>>, Path(env): Path<String>) -> Response {
    response(super::sync::synchronize(&state, env, false).await)
}
