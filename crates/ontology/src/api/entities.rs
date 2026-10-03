use std::path::PathBuf;

use axum::{
    extract::{Path, Query, State},
    routing::{get, put},
    Extension, Json, Router,
};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    api::{resolve_env, ListQuery},
    auth::Actor,
    database::text_revision::TextContent,
    domain::{AttributeKind, AttributeRole},
    error::AppError,
    http::AppState,
    system_catalog::DIRECTORY_TYPE_ID,
};

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/envs/:env/entities",
            get(list).post(super::entity_creation::create),
        )
        .route("/envs/:env/entities/:id", get(detail).patch(update))
        .route(
            "/envs/:env/entities/:id/attributes/:attribute",
            put(set_value),
        )
        .route(
            "/envs/:env/entities/:id/attributes/:attribute/text",
            put(set_text).get(text_history),
        )
        .route(
            "/envs/:env/entities/:id/attributes/:attribute/text/:revision",
            get(text_content),
        )
        .route(
            "/envs/:env/entities/:id/attributes/:attribute/nfs-path",
            put(set_nfs_path),
        )
}

async fn list(
    State(state): State<AppState>,
    Path(env): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<Json<Value>, AppError> {
    let env = resolve_env(&state, &env).await?;
    let limit = query.limit.unwrap_or(100).min(500);
    Ok(Json(
        serde_json::json!({"items":state.database.store().await.entities(env.env_num,query.include_deleted,query.offset,limit).await?}),
    ))
}
async fn detail(
    State(state): State<AppState>,
    Path((env, id)): Path<(String, Uuid)>,
) -> Result<Json<Value>, AppError> {
    let env = resolve_env(&state, &env).await?;
    let store = state.database.store().await;
    let item = store.entity(env.env_num, id).await?;
    let values = store.structured_values(env.env_num, id).await?;
    let definitions = store
        .attributes(env.env_num, Some(item.entity_type_id), false)
        .await?;
    let actions = store
        .actions(env.env_num, item.entity_type_id, false)
        .await?;
    let text_attributes = definitions
        .iter()
        .filter(|definition| definition.kind == AttributeKind::Text)
        .cloned()
        .map(|definition| {
            let store = store.clone();
            async move {
                let history = store.text_revisions(env.env_num, id, definition.id).await?;
                Ok::<_, AppError>(serde_json::json!({"definition": definition, "current": history.into_iter().find(|row| row["is_current"] == true && row["status"] == "ready")}))
            }
        });
    let mut text_rows = Vec::new();
    for future in text_attributes {
        text_rows.push(future.await?);
    }
    let needs_completion = !store.intrinsic_complete(env.env_num, id).await?;
    Ok(Json(
        serde_json::json!({"item":item,"attribute_definitions":definitions,"structured_attributes":values,"text_attributes":text_rows,"actions":actions,"needs_completion":needs_completion}),
    ))
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
    Path((env, id)): Path<(String, Uuid)>,
    Json(body): Json<Update>,
) -> Result<Json<Value>, AppError> {
    actor.require_manage()?;
    let env = resolve_env(&state, &env).await?;
    let store = state.database.store().await;
    let current = store.entity(env.env_num, id).await?;
    if current.entity_type_id == DIRECTORY_TYPE_ID {
        return Err(AppError::invalid(
            "directories must be updated through the directory API",
        ));
    }
    if !body.is_deleted {
        store
            .validate_required_values(env.env_num, &current)
            .await?;
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

#[derive(Deserialize)]
struct ValueBody {
    kind: AttributeKind,
    value: Value,
    #[serde(default)]
    is_deleted: bool,
    expected_revision: i64,
}
async fn set_value(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path((env, id, attribute)): Path<(String, Uuid, i64)>,
    Json(body): Json<ValueBody>,
) -> Result<Json<Value>, AppError> {
    actor.require_manage()?;
    let env = resolve_env(&state, &env).await?;
    let store = state.database.store().await;
    let entity = store.entity(env.env_num, id).await?;
    let definition = store
        .attributes(env.env_num, Some(entity.entity_type_id), true)
        .await?
        .into_iter()
        .find(|item| item.id == attribute)
        .ok_or(AppError::NotFound)?;
    if definition.is_deleted {
        return Err(AppError::invalid("attribute definition is inactive"));
    }
    if definition.required
        && (body.is_deleted || !crate::database::model::policy::nonempty(&body.value))
    {
        return Err(AppError::invalid("required attribute cannot be cleared"));
    }
    if definition.kind != body.kind {
        return Err(AppError::invalid(
            "attribute kind does not match its definition",
        ));
    }
    let item = store
        .set_structured_value(
            env.env_num,
            id,
            attribute,
            crate::database::model::inputs::StructuredWrite {
                kind: body.kind,
                value: &body.value,
                deleted: body.is_deleted,
                expected: body.expected_revision,
            },
            &actor.external_id,
        )
        .await?;
    Ok(Json(serde_json::json!({"item":item})))
}

#[derive(Deserialize)]
struct TextBody {
    format: String,
    content: String,
    expected_revision: i64,
}
async fn set_text(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path((env_key, id, attribute)): Path<(String, Uuid, i64)>,
    Json(body): Json<TextBody>,
) -> Result<Json<Value>, AppError> {
    actor.require_manage()?;
    let env = resolve_env(&state, &env_key).await?;
    let store = state.database.store().await;
    let entity = store.entity(env.env_num, id).await?;
    let definition = store
        .attributes(env.env_num, Some(entity.entity_type_id), false)
        .await?
        .into_iter()
        .find(|item| item.id == attribute && item.kind == AttributeKind::Text)
        .ok_or(AppError::NotFound)?;
    if definition.storage_mode != crate::domain::StorageMode::Markdown {
        return Err(AppError::invalid(
            "attribute is not configured for markdown storage",
        ));
    }
    if definition.attribute_role != AttributeRole::Custom && body.format != "md" {
        return Err(AppError::invalid("markdown attributes require md format"));
    }
    let result = store
        .save_text_revision(
            &state.text_store,
            crate::database::model::inputs::TextWrite {
                env: env.env_num,
                env_key: &env.env_key,
                entity: id,
                attribute,
                input: TextContent::Inline {
                    content: body.content.as_bytes(),
                    format: &body.format,
                },
                required: definition.required,
                expected: body.expected_revision,
            },
            &actor.external_id,
        )
        .await?;
    Ok(Json(serde_json::to_value(result).map_err(|_| {
        AppError::dependency("invalid text revision")
    })?))
}

#[derive(Deserialize)]
struct NfsPathBody {
    path: String,
    expected_revision: i64,
}

async fn set_nfs_path(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path((env_key, id, attribute)): Path<(String, Uuid, i64)>,
    Json(body): Json<NfsPathBody>,
) -> Result<Json<Value>, AppError> {
    actor.require_manage()?;
    let env = resolve_env(&state, &env_key).await?;
    let store = state.database.store().await;
    let entity = store.entity(env.env_num, id).await?;
    let definition = store
        .attributes(env.env_num, Some(entity.entity_type_id), false)
        .await?
        .into_iter()
        .find(|item| {
            item.id == attribute
                && item.attribute_role == crate::domain::AttributeRole::Ext
                && item.storage_mode == crate::domain::StorageMode::NfsPath
        })
        .ok_or(AppError::NotFound)?;
    let result = store
        .save_text_revision(
            &state.text_store,
            crate::database::model::inputs::TextWrite {
                env: env.env_num,
                env_key: &env.env_key,
                entity: id,
                attribute: definition.id,
                input: TextContent::NfsPath(&body.path),
                required: definition.required,
                expected: body.expected_revision,
            },
            &actor.external_id,
        )
        .await?;
    Ok(Json(serde_json::to_value(result).map_err(|_| {
        AppError::dependency("invalid text revision")
    })?))
}
async fn text_history(
    State(state): State<AppState>,
    Path((env, id, attribute)): Path<(String, Uuid, i64)>,
) -> Result<Json<Value>, AppError> {
    let env = resolve_env(&state, &env).await?;
    state.database.store().await.entity(env.env_num, id).await?;
    Ok(Json(
        serde_json::json!({"items":state.database.store().await.text_revisions(env.env_num,id,attribute).await?}),
    ))
}
async fn text_content(
    State(state): State<AppState>,
    Path((env, id, attribute, revision)): Path<(String, Uuid, i64, i64)>,
) -> Result<Json<Value>, AppError> {
    let env = resolve_env(&state, &env).await?;
    state.database.store().await.entity(env.env_num, id).await?;
    let (path, format) = state
        .database
        .store()
        .await
        .text_revision_path(env.env_num, id, attribute, revision)
        .await?;
    let content = if format == "nfs_path" {
        state.text_store.read_nfs(&PathBuf::from(path)).await?
    } else {
        state.text_store.read(&PathBuf::from(path)).await?
    };
    let (hash, bytes) = state
        .database
        .store()
        .await
        .text_revision_metadata(env.env_num, id, attribute, revision)
        .await?;
    if i64::try_from(content.len()).ok() != Some(bytes)
        || hex::encode(Sha256::digest(&content)) != hash
    {
        return Err(AppError::Conflict(
            "text content no longer matches its revision checksum; bind a new revision".into(),
        ));
    }
    let content =
        String::from_utf8(content).map_err(|_| AppError::dependency("stored text is not UTF-8"))?;
    Ok(Json(
        serde_json::json!({"format":format,"content":content,"revision":revision}),
    ))
}
