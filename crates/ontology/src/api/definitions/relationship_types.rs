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
            "/envs/:env/relationship-types",
            get(relationship_types).post(create_relationship_type),
        )
        .route(
            "/envs/:env/relationship-types/:type_id",
            patch(update_relationship_type),
        )
}

async fn relationship_types(
    State(state): State<AppState>,
    Path(env): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let env = resolve_env(&state, &env).await?;
    Ok(Json(
        serde_json::json!({"items":state.database.store().await.relationship_types(env.env_num, query.include_deleted).await?}),
    ))
}

#[derive(Deserialize)]
struct RelationshipTypeCreateInput {
    key: String,
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    source_entity_type_id: Option<Uuid>,
    target_entity_type_ids: Vec<Uuid>,
}

async fn create_relationship_type(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path(env): Path<String>,
    Json(body): Json<RelationshipTypeCreateInput>,
) -> Result<Json<serde_json::Value>, AppError> {
    actor.require_manage()?;
    if body.source_entity_type_id.is_none() {
        return Err(AppError::invalid(
            "relationship types require a source entity type",
        ));
    }
    let env = resolve_env(&state, &env).await?;
    let item = state
        .database
        .store()
        .await
        .create_relationship_type_config(
            env.env_num,
            crate::database::model::inputs::RelationshipTypeCreate {
                key: &body.key,
                title: &body.name,
                description: &body.description,
                source: body.source_entity_type_id,
                targets: &body.target_entity_type_ids,
            },
            &actor.external_id,
        )
        .await?;
    Ok(Json(serde_json::json!({"item":item})))
}

#[derive(Deserialize)]
struct RelationshipTypeUpdateInput {
    name: String,
    #[serde(default)]
    description: String,
    is_deleted: bool,
    expected_revision: i64,
    #[serde(default)]
    source_entity_type_id: Option<Uuid>,
    target_entity_type_ids: Vec<Uuid>,
}

async fn update_relationship_type(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path((env, id)): Path<(String, Uuid)>,
    Json(body): Json<RelationshipTypeUpdateInput>,
) -> Result<Json<serde_json::Value>, AppError> {
    actor.require_manage()?;
    let env = resolve_env(&state, &env).await?;
    let revision = state
        .database
        .store()
        .await
        .update_relationship_type_with_scope(
            env.env_num,
            id,
            crate::database::model::inputs::RelationshipTypeUpdate {
                title: &body.name,
                description: &body.description,
                source: body.source_entity_type_id,
                targets: &body.target_entity_type_ids,
                deleted: body.is_deleted,
                expected: body.expected_revision,
            },
            &actor.external_id,
        )
        .await?
        .revision;
    Ok(Json(serde_json::json!({"revision":revision})))
}
