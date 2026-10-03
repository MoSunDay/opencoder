use crate::{
    api::{resolve_env, ListQuery},
    auth::Actor,
    error::AppError,
    http::AppState,
};
use axum::{
    extract::{Path, Query, State},
    routing::{get, patch},
    Extension, Json, Router,
};
use serde::Deserialize;
use uuid::Uuid;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/envs/:env/entity-types/:type_id/actions",
            get(actions).post(create_action),
        )
        .route("/envs/:env/actions/:action_id", patch(update_action))
}

async fn actions(
    State(state): State<AppState>,
    Path((env, type_id)): Path<(String, Uuid)>,
    Query(query): Query<ListQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let env = resolve_env(&state, &env).await?;
    Ok(Json(
        serde_json::json!({"items": state.database.store().await.actions(env.env_num, type_id, query.include_deleted).await?}),
    ))
}

#[derive(Deserialize)]
struct ActionCreate {
    operation_type: String,
    operation: String,
    #[serde(default)]
    description: String,
}

async fn create_action(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path((env, type_id)): Path<(String, Uuid)>,
    Json(body): Json<ActionCreate>,
) -> Result<Json<serde_json::Value>, AppError> {
    actor.require_manage()?;
    let env = resolve_env(&state, &env).await?;
    let item = state
        .database
        .store()
        .await
        .create_action(
            env.env_num,
            type_id,
            &body.operation_type,
            &body.operation,
            &body.description,
            &actor.external_id,
        )
        .await?;
    Ok(Json(serde_json::json!({"item": item})))
}

#[derive(Deserialize)]
struct ActionUpdate {
    #[serde(default)]
    description: String,
    is_deleted: bool,
    expected_revision: i64,
}

async fn update_action(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path((env, id)): Path<(String, i64)>,
    Json(body): Json<ActionUpdate>,
) -> Result<Json<serde_json::Value>, AppError> {
    actor.require_manage()?;
    let env = resolve_env(&state, &env).await?;
    let item = state
        .database
        .store()
        .await
        .update_action(
            env.env_num,
            id,
            &body.description,
            body.is_deleted,
            body.expected_revision,
            &actor.external_id,
        )
        .await?;
    Ok(Json(serde_json::json!({"item": item})))
}
