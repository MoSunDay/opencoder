use super::{
    super::{
        store::{check_revision, name, numeric_id},
        Store,
    },
    policy::validate_attribute,
};
use crate::{
    domain::{AttributeDefinition, AttributeKind, AttributeRole, EntityType, StorageMode},
    error::AppError,
};
use uuid::Uuid;

impl Store {
    pub(crate) async fn attributes(
        &self,
        env: i64,
        kind: Option<Uuid>,
        deleted: bool,
    ) -> Result<Vec<AttributeDefinition>, AppError> {
        Ok(self
            .list::<AttributeDefinition>("attribute_definitions", env)
            .await?
            .into_iter()
            .filter(|item| {
                kind.is_none_or(|id| item.entity_type_id == id) && (deleted || !item.is_deleted)
            })
            .collect())
    }

    pub(crate) async fn ensure_intrinsic_attributes(
        &self,
        env: i64,
        kind: Uuid,
        actor: &str,
    ) -> Result<(), AppError> {
        for (key, title, role) in [
            ("source", "来源", AttributeRole::Source),
            ("ext", "拓展信息", AttributeRole::Ext),
        ] {
            if !self
                .attributes(env, Some(kind), true)
                .await?
                .iter()
                .any(|item| item.attribute_role == role)
            {
                self.create_attribute_config(
                    env,
                    kind,
                    crate::database::model::inputs::AttributeCreate {
                        key,
                        title,
                        description: "",
                        value_kind: AttributeKind::Text,
                        role,
                        storage: StorageMode::Markdown,
                        required: true,
                    },
                    actor,
                )
                .await?;
            }
        }
        Ok(())
    }

    pub(crate) async fn create_attribute_config(
        &self,
        env: i64,
        kind: Uuid,
        input: crate::database::model::inputs::AttributeCreate<'_>,
        actor: &str,
    ) -> Result<AttributeDefinition, AppError> {
        let crate::database::model::inputs::AttributeCreate {
            key,
            title,
            description,
            value_kind,
            role,
            storage,
            required,
        } = input;
        let entity_type: EntityType = self.get("entity_types", env, kind).await?;
        if entity_type.is_deleted {
            return Err(AppError::invalid("entity type is inactive"));
        }
        let key = crate::domain::validate_key(key, "attribute key")?;
        validate_attribute(&value_kind, &role, &storage, required)?;
        let definitions = self.attributes(env, Some(kind), true).await?;
        if definitions.iter().any(|item| {
            item.attribute_key == key
                || (role != AttributeRole::Custom && item.attribute_role == role)
        }) {
            return Err(AppError::Conflict(
                "attribute key or intrinsic role already exists".into(),
            ));
        }
        if (key == "source" && role != AttributeRole::Source)
            || (key == "ext" && role != AttributeRole::Ext)
        {
            return Err(AppError::invalid("source/ext keys are reserved"));
        }
        let item = AttributeDefinition {
            id: numeric_id(),
            env_num: env,
            entity_type_id: kind,
            attribute_key: key,
            name: name(title)?,
            description: description.trim().into(),
            kind: value_kind,
            attribute_role: role,
            storage_mode: storage,
            required,
            revision: 1,
            is_deleted: false,
        };
        self.put("attribute_definitions", env, item.id, &item)
            .await?;
        self.audit(env, actor, "attribute.create", item.id, 1)
            .await?;
        Ok(item)
    }

    pub(crate) async fn update_attribute(
        &self,
        env: i64,
        id: i64,
        input: crate::database::model::inputs::AttributeUpdate<'_>,
        actor: &str,
    ) -> Result<(), AppError> {
        let crate::database::model::inputs::AttributeUpdate {
            title,
            description,
            required,
            role,
            storage,
            deleted,
            expected,
        } = input;
        let mut item: AttributeDefinition = self.get("attribute_definitions", env, id).await?;
        check_revision(item.revision, expected)?;
        if role.is_some_and(|role| role != item.attribute_role) {
            return Err(AppError::invalid("attribute role cannot change"));
        }
        if item.attribute_role != AttributeRole::Custom && deleted {
            return Err(AppError::invalid("source/ext cannot be deleted"));
        }
        let storage = storage.unwrap_or(item.storage_mode);
        validate_attribute(&item.kind, &item.attribute_role, &storage, required)?;
        item.name = name(title)?;
        item.description = description.trim().into();
        item.required = required;
        item.is_deleted = deleted;
        item.storage_mode = storage;
        item.revision += 1;
        self.put("attribute_definitions", env, id, &item).await?;
        self.audit(env, actor, "attribute.update", id, item.revision)
            .await
    }
}
