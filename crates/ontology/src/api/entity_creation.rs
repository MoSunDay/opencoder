use crate::{
    api::resolve_env,
    auth::Actor,
    database::text_revision::{validate_text, TextContent},
    domain::{AttributeKind, AttributeRole, StorageMode},
    error::AppError,
    http::AppState,
    text_store::TextStore,
};
use axum::{
    extract::{Path, State},
    Extension, Json,
};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;
#[derive(Deserialize, serde::Serialize)]
pub(super) struct Create {
    request_id: Uuid,
    directory_id: Option<Uuid>,
    entity_type_id: Uuid,
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    ext: Option<String>,
    #[serde(default)]
    attributes: std::collections::BTreeMap<i64, Value>,
}
#[allow(clippy::too_many_lines)]
pub(super) async fn create(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path(env): Path<String>,
    Json(body): Json<Create>,
) -> Result<Json<Value>, AppError> {
    actor.require_manage()?;
    if body.name.trim().is_empty() {
        return Err(AppError::invalid("entity name is required"));
    }
    let env = resolve_env(&state, &env).await?;
    let store = state.database.store().await.for_creation();
    let fingerprint = hex::encode(Sha256::digest(
        serde_json::to_vec(&(&actor.external_id, &body))
            .map_err(|_| AppError::invalid("invalid creation request"))?,
    ));
    let creation_id = store
        .claim_creation(
            env.env_num,
            body.request_id,
            &fingerprint,
            &actor.external_id,
        )
        .await?;
    match store.entity(env.env_num, creation_id).await {
        Ok(item) if item.revision > 0 => return Ok(Json(serde_json::json!({"item":item}))),
        Ok(_) | Err(AppError::NotFound) => {}
        Err(error) => return Err(error),
    }

    if let Some(directory) = body.directory_id {
        let parent = store.entity(env.env_num, directory).await?;
        if parent.entity_type_id != crate::system_catalog::DIRECTORY_TYPE_ID
            || parent.is_deleted
            || parent.revision == 0
        {
            return Err(AppError::invalid(
                "directory must be an active directory entity",
            ));
        }
    }
    let definitions = store
        .attributes(env.env_num, Some(body.entity_type_id), false)
        .await?;
    if body.source.as_deref().unwrap_or("").trim().is_empty()
        || body.ext.as_deref().unwrap_or("").trim().is_empty()
    {
        return Err(AppError::invalid("source and ext are required"));
    }
    for role in [AttributeRole::Source, AttributeRole::Ext] {
        if !definitions.iter().any(|d| d.attribute_role == role) {
            return Err(AppError::dependency(
                "entity type intrinsic definitions are missing",
            ));
        }
    }
    let mut prepared_text = Vec::new();
    for definition in &definitions {
        let value = match definition.attribute_role {
            AttributeRole::Source => body.source.clone().map(Value::String),
            AttributeRole::Ext => body.ext.clone().map(Value::String),
            AttributeRole::Custom => body.attributes.get(&definition.id).cloned(),
        };
        if definition.required
            && value
                .as_ref()
                .is_none_or(|value| !crate::database::model::policy::nonempty(value))
        {
            return Err(AppError::invalid(format!(
                "required attribute {} is missing",
                definition.attribute_key
            )));
        }
        if let Some(value) = value {
            if definition.kind == AttributeKind::Text {
                let text = value
                    .as_str()
                    .ok_or_else(|| AppError::invalid("text attribute must be a string"))?;
                if definition.storage_mode == StorageMode::NfsPath {
                    let path = TextStore::nfs_relative_path(text)?;
                    let content = state.text_store.read_nfs(&path).await?;
                    validate_text(&content, definition.required)?;
                    prepared_text.push((definition.id, definition.required, Some(path), content));
                } else {
                    validate_text(text.as_bytes(), definition.required)?;
                    prepared_text.push((
                        definition.id,
                        definition.required,
                        None,
                        text.as_bytes().to_vec(),
                    ));
                }
            } else {
                crate::database::validate_structured_value(&definition.kind, &value)?;
            }
        }
    }
    if body.attributes.keys().any(|id| {
        !definitions
            .iter()
            .any(|d| d.id == *id && d.attribute_role == AttributeRole::Custom)
    }) {
        return Err(AppError::invalid(
            "unknown or intrinsic attribute in attributes map",
        ));
    }
    let item = store
        .stage_entity(
            env.env_num,
            creation_id,
            body.entity_type_id,
            &body.name,
            &body.description,
            &actor.external_id,
        )
        .await?;
    for (attribute, required, path, content) in prepared_text {
        let history = store
            .text_revisions(env.env_num, item.id, attribute)
            .await?;
        let current = history
            .iter()
            .find(|row| row["is_current"] == true && row["status"] == "ready");
        if let Some(row) = current {
            if row["sha256"] != hex::encode(Sha256::digest(&content)) {
                return Err(AppError::Conflict(
                    "creation content changed during retry".into(),
                ));
            }
            continue;
        }
        let path = path.map(|path| path.to_string_lossy().to_string());
        let input = match path.as_deref() {
            Some(path) => TextContent::PreparedNfs {
                path,
                content: &content,
            },
            None => TextContent::Inline {
                content: &content,
                format: "md",
            },
        };
        store
            .save_text_revision(
                &state.text_store,
                crate::database::model::inputs::TextWrite {
                    env: env.env_num,
                    env_key: &env.env_key,
                    entity: item.id,
                    attribute,
                    input,
                    required,
                    expected: 0,
                },
                &actor.external_id,
            )
            .await?;
    }
    let stored_values = store.structured_values(env.env_num, item.id).await?;
    for definition in definitions.iter().filter(|d| d.kind != AttributeKind::Text) {
        if let Some(value) = body.attributes.get(&definition.id) {
            if stored_values.iter().any(|row| {
                row["attribute_definition_id"]
                    .as_str()
                    .and_then(|id| id.parse::<i64>().ok())
                    == Some(definition.id)
            }) {
                continue;
            }
            store
                .set_structured_value(
                    env.env_num,
                    item.id,
                    definition.id,
                    crate::database::model::inputs::StructuredWrite {
                        kind: definition.kind.clone(),
                        value,
                        deleted: false,
                        expected: 0,
                    },
                    &actor.external_id,
                )
                .await?;
        }
    }
    store.validate_required_values(env.env_num, &item).await?;
    if let Some(directory) = body.directory_id {
        let membership = Uuid::from_u128(0x0000_0000_0000_4000_8000_0000_0000_0010);
        if !store
            .relationships(env.env_num, Some(item.id), false)
            .await?
            .iter()
            .any(|row| row.relationship_type_id == membership && row.target_entity_id == directory)
        {
            store
                .create_relationship(
                    env.env_num,
                    membership,
                    item.id,
                    directory,
                    "目录归属",
                    &actor.external_id,
                )
                .await?;
        }
    }
    let item = store
        .publish_entity(env.env_num, item.id, &actor.external_id)
        .await?;
    Ok(Json(serde_json::json!({"item":item})))
}
