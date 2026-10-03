use crate::domain::{AttributeKind, AttributeRole, StorageMode};
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
            "/envs/:env/entity-types/:type_id/attributes",
            get(attributes).post(create_attribute),
        )
        .route(
            "/envs/:env/attributes/:attribute_id",
            patch(update_attribute),
        )
}

async fn attributes(
    State(state): State<AppState>,
    Path((env, type_id)): Path<(String, Uuid)>,
    Query(query): Query<ListQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let env = resolve_env(&state, &env).await?;
    Ok(Json(
        serde_json::json!({"items":state.database.store().await.attributes(env.env_num, Some(type_id), query.include_deleted).await?}),
    ))
}

#[derive(Deserialize)]
struct AttributeCreate {
    key: String,
    name: String,
    #[serde(default)]
    description: String,
    kind: AttributeKind,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    attribute_role: Option<AttributeRole>,
    #[serde(default)]
    storage_mode: Option<StorageMode>,
}

async fn create_attribute(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path((env, type_id)): Path<(String, Uuid)>,
    Json(body): Json<AttributeCreate>,
) -> Result<Json<serde_json::Value>, AppError> {
    actor.require_manage()?;
    let env = resolve_env(&state, &env).await?;
    let storage_mode = body
        .storage_mode
        .unwrap_or(if body.kind == AttributeKind::Text {
            StorageMode::Markdown
        } else {
            StorageMode::Sql
        });
    let item = state
        .database
        .store()
        .await
        .create_attribute_config(
            env.env_num,
            type_id,
            crate::database::model::inputs::AttributeCreate {
                key: &body.key,
                title: &body.name,
                description: &body.description,
                value_kind: body.kind,
                role: body.attribute_role.unwrap_or(AttributeRole::Custom),
                storage: storage_mode,
                required: body.required,
            },
            &actor.external_id,
        )
        .await?;
    Ok(Json(serde_json::json!({"item":item})))
}

#[derive(Deserialize)]
struct AttributeUpdate {
    name: String,
    #[serde(default)]
    description: String,
    required: bool,
    is_deleted: bool,
    expected_revision: i64,
    #[serde(default)]
    attribute_role: Option<AttributeRole>,
    #[serde(default)]
    storage_mode: Option<StorageMode>,
}

async fn update_attribute(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path((env, id)): Path<(String, i64)>,
    Json(body): Json<AttributeUpdate>,
) -> Result<Json<serde_json::Value>, AppError> {
    actor.require_manage()?;
    let env = resolve_env(&state, &env).await?;
    state
        .database
        .store()
        .await
        .update_attribute(
            env.env_num,
            id,
            crate::database::model::inputs::AttributeUpdate {
                title: &body.name,
                description: &body.description,
                required: body.required,
                role: body.attribute_role,
                storage: body.storage_mode,
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
