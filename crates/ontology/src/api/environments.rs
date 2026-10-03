use axum::{
    extract::{Query, State},
    routing::{get, patch},
    Extension, Json, Router,
};
use serde::Deserialize;

use crate::{api::ListQuery, auth::Actor, error::AppError, http::AppState};

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/environments", get(list).post(create))
        .route("/environments/:env", patch(update))
}

async fn list(
    State(state): State<AppState>,
    Query(query): Query<ListQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    Ok(Json(
        serde_json::json!({"items":state.database.store().await.environments(query.include_deleted).await?}),
    ))
}

#[derive(Deserialize)]
struct Create {
    key: String,
    name: String,
    #[serde(default)]
    description: String,
}
async fn create(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Json(body): Json<Create>,
) -> Result<Json<serde_json::Value>, AppError> {
    actor.require_manage()?;
    let item = state
        .database
        .store()
        .await
        .create_environment(&body.key, &body.name, &body.description, &actor.external_id)
        .await?;
    Ok(Json(serde_json::json!({"item":item})))
}

#[derive(Deserialize)]
struct Update {
    name: String,
    #[serde(default)]
    description: String,
    is_deleted: bool,
    expected_revision: i64,
}
async fn update(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    axum::extract::Path(env): axum::extract::Path<String>,
    Json(body): Json<Update>,
) -> Result<Json<serde_json::Value>, AppError> {
    actor.require_manage()?;
    if env == "debug" && body.is_deleted {
        return Err(AppError::invalid("debug ENV cannot be deleted"));
    }
    let item = state
        .database
        .store()
        .await
        .update_environment(
            &env,
            &body.name,
            &body.description,
            body.is_deleted,
            body.expected_revision,
            &actor.external_id,
        )
        .await?;
    Ok(Json(serde_json::json!({"item":item})))
}
