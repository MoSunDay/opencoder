use crate::{api::resolve_env, auth::Actor, error::AppError, http::AppState};
use axum::{
    extract::{Path, State},
    routing::{get, post, put},
    Extension, Json, Router,
};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/envs/:env/vectors/:id", get(get_vector))
        .route(
            "/envs/:env/entities/:entity/vectors/:attribute",
            put(set_vector),
        )
        .route("/envs/:env/vector-search", post(search))
}
#[derive(Deserialize)]
struct SetBody {
    vector: Vec<f32>,
    vector_id: Option<Uuid>,
    #[serde(default)]
    is_deleted: bool,
    expected_revision: i64,
}
async fn set_vector(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path((env, entity, attribute)): Path<(String, Uuid, i64)>,
    Json(body): Json<SetBody>,
) -> Result<Json<Value>, AppError> {
    actor.require_manage()?;
    let env = resolve_env(&state, &env).await?;
    let store = state.database.store().await;
    let entity_row = store.entity(env.env_num, entity).await?;
    let definition = store
        .attributes(env.env_num, Some(entity_row.entity_type_id), false)
        .await?
        .into_iter()
        .find(|item| item.id == attribute && item.kind == crate::domain::AttributeKind::Vector)
        .ok_or(AppError::NotFound)?;
    let id = store
        .set_vector(
            env.env_num,
            entity,
            definition.id,
            crate::database::model::inputs::VectorWrite {
                requested: body.vector_id,
                values: &body.vector,
                deleted: body.is_deleted,
                expected: body.expected_revision,
            },
            &actor.external_id,
        )
        .await?;
    Ok(Json(serde_json::json!({"vector_id":id})))
}
async fn get_vector(
    State(state): State<AppState>,
    Path((env, id)): Path<(String, Uuid)>,
) -> Result<Json<Value>, AppError> {
    let env = resolve_env(&state, &env).await?;
    Ok(Json(
        state.database.store().await.vector(env.env_num, id).await?,
    ))
}
#[derive(Deserialize)]
struct Search {
    attribute_definition_id: i64,
    vector: Vec<f32>,
    limit: Option<u32>,
}
async fn search(
    State(state): State<AppState>,
    Path(env): Path<String>,
    Json(body): Json<Search>,
) -> Result<Json<Value>, AppError> {
    let env = resolve_env(&state, &env).await?;
    let items = state
        .database
        .store()
        .await
        .search_vectors(
            env.env_num,
            body.attribute_definition_id,
            &body.vector,
            body.limit.unwrap_or(20).min(100),
        )
        .await?;
    Ok(Json(serde_json::json!({"items":items})))
}
