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
            "/envs/:env/entity-types",
            get(entity_types).post(create_entity_type),
        )
        .route(
            "/envs/:env/entity-types/:type_id",
            patch(update_entity_type),
        )
}

async fn entity_types(
    State(state): State<AppState>,
    Path(env): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let env = resolve_env(&state, &env).await?;
    Ok(Json(
        serde_json::json!({"items":state.database.store().await.entity_types(env.env_num, query.include_deleted).await?}),
    ))
}

#[derive(Deserialize)]
struct EntityTypeCreateInput {
    key: String,
    name: String,
    #[serde(default)]
    description: String,
}

async fn create_entity_type(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path(env): Path<String>,
    Json(body): Json<EntityTypeCreateInput>,
) -> Result<Json<serde_json::Value>, AppError> {
    actor.require_manage()?;
    let env = resolve_env(&state, &env).await?;
    let item = state
        .database
        .store()
        .await
        .create_entity_type(
            env.env_num,
            &body.key,
            &body.name,
            &body.description,
            &actor.external_id,
        )
        .await?;
    Ok(Json(serde_json::json!({"item":item})))
}

#[derive(Deserialize)]
struct EntityTypeUpdateInput {
    name: String,
    #[serde(default)]
    description: String,
    is_deleted: bool,
    expected_revision: i64,
}

async fn update_entity_type(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path((env, id)): Path<(String, Uuid)>,
    Json(body): Json<EntityTypeUpdateInput>,
) -> Result<Json<serde_json::Value>, AppError> {
    actor.require_manage()?;
    let env = resolve_env(&state, &env).await?;
    state
        .database
        .store()
        .await
        .update_entity_type(
            env.env_num,
            id,
            crate::database::model::inputs::DefinitionUpdate {
                title: &body.name,
                description: &body.description,
                deleted: body.is_deleted,
                expected: body.expected_revision,
            },
            &actor.external_id,
        )
        .await?;
    Ok(Json(
        serde_json::json!({"revision":body.expected_revision+1}),
    ))
}
