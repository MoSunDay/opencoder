use std::collections::HashMap;

use crate::{
    api::{resolve_env, ListQuery},
    auth::Actor,
    domain::validate_directory_move,
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

const DIRECTORY_TYPE: Uuid = Uuid::from_u128(0x0000_0000_0000_4000_8000_0000_0000_0001);
const BELONGS_TO: Uuid = Uuid::from_u128(0x0000_0000_0000_4000_8000_0000_0000_0010);
const ROOT_DIRECTORY: Uuid = Uuid::from_u128(0x0000_0000_0000_4000_8000_0000_0000_0002);

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/envs/:env/directories/tree",
            get(directory_tree).post(create_directory),
        )
        .route("/envs/:env/directories/:id/move", patch(move_directory))
        .route("/envs/:env/directories/:id", patch(update_directory))
}

async fn directory_tree(
    State(state): State<AppState>,
    Path(env): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<Json<Value>, AppError> {
    let env = resolve_env(&state, &env).await?;
    let store = state.database.store().await;
    let mut entities = store
        .entities(env.env_num, query.include_deleted, 0, u32::MAX)
        .await?;
    entities.retain(|e| e.entity_type_id == DIRECTORY_TYPE);
    let parents = store
        .relationships(env.env_num, None, query.include_deleted)
        .await?
        .into_iter()
        .filter(|e| e.relationship_type_id == BELONGS_TO)
        .map(|e| (e.source_entity_id, e.target_entity_id))
        .collect::<HashMap<_, _>>();
    let items=entities.into_iter().map(|e|serde_json::json!({"id":e.id,"name":e.name,"description":e.description,"parent_id":parents.get(&e.id),"revision":e.revision,"is_deleted":e.is_deleted})).collect::<Vec<_>>();
    Ok(Json(
        serde_json::json!({"items":items,"root_id":ROOT_DIRECTORY}),
    ))
}

#[derive(Deserialize)]
struct DirectoryCreate {
    name: String,
    #[serde(default)]
    description: String,
    parent_id: Option<Uuid>,
}
async fn create_directory(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path(env): Path<String>,
    Json(body): Json<DirectoryCreate>,
) -> Result<Json<Value>, AppError> {
    actor.require_manage()?;
    let env = resolve_env(&state, &env).await?;
    let store = state.database.store().await;
    let item = store
        .create_entity(
            env.env_num,
            DIRECTORY_TYPE,
            &body.name,
            &body.description,
            &actor.external_id,
        )
        .await?;
    let parent = body.parent_id.unwrap_or(ROOT_DIRECTORY);
    validate_membership(&store, env.env_num, item.id, parent, false).await?;
    store
        .create_relationship(
            env.env_num,
            BELONGS_TO,
            item.id,
            parent,
            "目录归属",
            &actor.external_id,
        )
        .await?;
    Ok(Json(serde_json::json!({"item":item})))
}

#[derive(Deserialize)]
struct Move {
    parent_id: Uuid,
}
async fn move_directory(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path((env, id)): Path<(String, Uuid)>,
    Json(body): Json<Move>,
) -> Result<Json<Value>, AppError> {
    actor.require_manage()?;
    if id == ROOT_DIRECTORY {
        return Err(AppError::invalid("root directory cannot be moved"));
    }
    let env = resolve_env(&state, &env).await?;
    let store = state.database.store().await;
    if store.entity(env.env_num, id).await?.entity_type_id != DIRECTORY_TYPE {
        return Err(AppError::NotFound);
    }
    validate_membership(&store, env.env_num, id, body.parent_id, true).await?;
    let old = store
        .relationships(env.env_num, Some(id), false)
        .await?
        .into_iter()
        .find(|r| r.relationship_type_id == BELONGS_TO && r.source_entity_id == id)
        .ok_or_else(|| AppError::dependency("directory membership is missing"))?;
    store
        .update_relationship(
            env.env_num,
            old.id,
            crate::database::model::inputs::RelationshipUpdate {
                description: &old.description,
                deleted: true,
                pinned: None,
                expected: old.revision,
            },
            &actor.external_id,
        )
        .await?;
    let relation = store
        .create_relationship(
            env.env_num,
            BELONGS_TO,
            id,
            body.parent_id,
            "目录归属",
            &actor.external_id,
        )
        .await?;
    Ok(Json(serde_json::json!({"relationship":relation})))
}

pub(super) async fn validate_membership(
    store: &crate::database::Store,
    env: i64,
    source: Uuid,
    target: Uuid,
    allow_existing: bool,
) -> Result<(), AppError> {
    let target_entity = store.entity(env, target).await?;
    if target_entity.entity_type_id != DIRECTORY_TYPE || target_entity.is_deleted {
        return Err(AppError::invalid(
            "membership target must be an active directory",
        ));
    }
    if !allow_existing
        && store
            .relationships(env, Some(source), false)
            .await?
            .iter()
            .any(|r| r.relationship_type_id == BELONGS_TO && r.source_entity_id == source)
    {
        return Err(AppError::Conflict(
            "entity already belongs to a directory".into(),
        ));
    }
    let source_entity = store.entity(env, source).await?;
    if source_entity.entity_type_id == DIRECTORY_TYPE {
        let parents: HashMap<Uuid, Uuid> = store
            .relationships(env, None, false)
            .await?
            .into_iter()
            .filter(|r| r.relationship_type_id == BELONGS_TO)
            .map(|r| (r.source_entity_id, r.target_entity_id))
            .collect();
        validate_directory_move(source, target, &parents)?;
    }
    Ok(())
}

#[derive(Deserialize)]
struct DirectoryUpdate {
    name: String,
    #[serde(default)]
    description: String,
    is_deleted: bool,
    expected_revision: i64,
}
async fn update_directory(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path((env, id)): Path<(String, Uuid)>,
    Json(body): Json<DirectoryUpdate>,
) -> Result<Json<Value>, AppError> {
    actor.require_manage()?;
    if id == ROOT_DIRECTORY && body.is_deleted {
        return Err(AppError::invalid("root directory cannot be deleted"));
    }
    let env = resolve_env(&state, &env).await?;
    let store = state.database.store().await;
    let item = store.entity(env.env_num, id).await?;
    if item.entity_type_id != DIRECTORY_TYPE {
        return Err(AppError::NotFound);
    }
    let has_children = store
        .relationships(env.env_num, Some(id), false)
        .await?
        .iter()
        .any(|relation| {
            relation.relationship_type_id == BELONGS_TO && relation.target_entity_id == id
        });
    if body.is_deleted && has_children {
        return Err(AppError::Conflict(
            "non-empty directory cannot be deleted".into(),
        ));
    }
    let item = store
        .update_entity(
            env.env_num,
            id,
            crate::database::model::inputs::EntityUpdate {
                title: &body.name,
                description: &body.description,
                deleted: body.is_deleted,
                expected: body.expected_revision,
            },
            &actor.external_id,
        )
        .await?;
    Ok(Json(serde_json::json!({"item":item})))
}
