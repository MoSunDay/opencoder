use axum::{
    extract::{Path, Query, State},
    routing::get,
    Extension, Json, Router,
};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::{
    api::{resolve_env, ListQuery},
    auth::Actor,
    domain::AspectDefaults,
    error::AppError,
    http::AppState,
};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/envs/:env/graph-aspects", get(list).post(create))
        .route(
            "/envs/:env/graph-aspects/:id",
            get(detail).patch(update).delete(soft_delete),
        )
}

async fn list(
    State(state): State<AppState>,
    Path(env): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<Json<Value>, AppError> {
    let env = resolve_env(&state, &env).await?;
    Ok(Json(
        serde_json::json!({"items":state.database.store().await.graph_aspects(env.env_num,query.include_deleted).await?}),
    ))
}

#[derive(Deserialize)]
struct Create {
    aspect_key: Option<String>,
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    entity_type_ids: Vec<Uuid>,
    #[serde(default)]
    relationship_type_ids: Vec<Uuid>,
    #[serde(flatten)]
    defaults: AspectDefaults,
}

async fn create(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path(env): Path<String>,
    Json(body): Json<Create>,
) -> Result<Json<Value>, AppError> {
    actor.require_manage()?;
    let env = resolve_env(&state, &env).await?;
    // 未提供 key（或只有空白）时生成随机 key，避免前端必须先起名才能保存切面。
    let aspect_key = match body.aspect_key.as_deref().map(str::trim) {
        Some(key) if !key.is_empty() => key.to_owned(),
        _ => format!("aspect-{}", &Uuid::new_v4().simple().to_string()[..12]),
    };
    let item = state
        .database
        .store()
        .await
        .create_graph_aspect(
            env.env_num,
            crate::database::model::inputs::AspectCreate {
                key: &aspect_key,
                title: &body.name,
                description: &body.description,
                types: body.entity_type_ids,
                relations: body.relationship_type_ids,
                defaults: body.defaults,
            },
            &actor.external_id,
        )
        .await?;
    Ok(Json(serde_json::json!({"item":item})))
}

async fn detail(
    State(state): State<AppState>,
    Path((env, id)): Path<(String, Uuid)>,
) -> Result<Json<Value>, AppError> {
    let env = resolve_env(&state, &env).await?;
    let item = state
        .database
        .store()
        .await
        .graph_aspect(env.env_num, id)
        .await?;
    Ok(Json(serde_json::json!({"item":item})))
}

#[derive(Deserialize)]
struct Update {
    name: String,
    #[serde(default)]
    description: String,
    entity_type_ids: Vec<Uuid>,
    relationship_type_ids: Vec<Uuid>,
    #[serde(flatten)]
    defaults: AspectDefaults,
    is_deleted: bool,
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
        .update_graph_aspect(
            env.env_num,
            id,
            crate::database::model::inputs::AspectUpdate {
                title: &body.name,
                description: &body.description,
                types: body.entity_type_ids,
                relations: body.relationship_type_ids,
                defaults: body.defaults,
                deleted: body.is_deleted,
                expected: body.expected_revision,
            },
            &actor.external_id,
        )
        .await?;
    Ok(Json(serde_json::json!({"item":item})))
}

#[derive(Deserialize)]
struct DeleteQuery {
    expected_revision: i64,
}

async fn soft_delete(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path((env, id)): Path<(String, Uuid)>,
    Query(query): Query<DeleteQuery>,
) -> Result<Json<Value>, AppError> {
    actor.require_manage()?;
    let env = resolve_env(&state, &env).await?;
    let item = state
        .database
        .store()
        .await
        .soft_delete_graph_aspect(env.env_num, id, query.expected_revision, &actor.external_id)
        .await?;
    Ok(Json(serde_json::json!({"item":item})))
}
