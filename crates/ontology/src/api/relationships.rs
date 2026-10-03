use crate::{
    api::{directories::validate_membership, resolve_env},
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
use serde_json::Value;
use uuid::Uuid;

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/envs/:env/relationships", get(list).post(create))
        .route("/envs/:env/relationships/:id", patch(update))
}

#[derive(Deserialize)]
struct RelationQuery {
    anchor: Option<Uuid>,
    #[serde(default)]
    include_deleted: bool,
}
async fn list(
    State(state): State<AppState>,
    Path(env): Path<String>,
    Query(query): Query<RelationQuery>,
) -> Result<Json<Value>, AppError> {
    let env = resolve_env(&state, &env).await?;
    Ok(Json(
        serde_json::json!({"items":state.database.store().await.relationships(env.env_num,query.anchor,query.include_deleted).await?}),
    ))
}
#[derive(Deserialize)]
struct Create {
    relationship_type_id: Uuid,
    source_entity_id: Uuid,
    target_entity_id: Uuid,
    #[serde(default)]
    description: String,
}
async fn create(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path(env): Path<String>,
    Json(body): Json<Create>,
) -> Result<Json<Value>, AppError> {
    actor.require_manage()?;
    let env = resolve_env(&state, &env).await?;
    let store = state.database.store().await;
    let relation_type = store
        .relationship_types(env.env_num, false)
        .await?
        .into_iter()
        .find(|v| v.id == body.relationship_type_id)
        .ok_or(AppError::NotFound)?;
    if relation_type.is_directory_membership {
        validate_membership(
            &store,
            env.env_num,
            body.source_entity_id,
            body.target_entity_id,
            false,
        )
        .await?;
    }
    let item = store
        .create_relationship(
            env.env_num,
            body.relationship_type_id,
            body.source_entity_id,
            body.target_entity_id,
            &body.description,
            &actor.external_id,
        )
        .await?;
    Ok(Json(serde_json::json!({"item":item})))
}
#[derive(Deserialize)]
struct Update {
    #[serde(default)]
    description: String,
    is_deleted: bool,
    is_pinned: Option<bool>,
    expected_revision: i64,
}
async fn update(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path((env, id)): Path<(String, Uuid)>,
    Json(body): Json<Update>,
) -> Result<Json<Value>, AppError> {
    actor.require_manage()?;
    let env = resolve_env(&state, &env).await?;
    let item = state
        .database
        .store()
        .await
        .update_relationship(
            env.env_num,
            id,
            crate::database::model::inputs::RelationshipUpdate {
                description: &body.description,
                deleted: body.is_deleted,
                pinned: body.is_pinned,
                expected: body.expected_revision,
            },
            &actor.external_id,
        )
        .await?;
    Ok(Json(serde_json::json!({"item":item})))
}
