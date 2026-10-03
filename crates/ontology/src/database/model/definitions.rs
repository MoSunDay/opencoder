use super::super::{
    store::{check_revision, name, numeric_id},
    Store,
};
use crate::{
    domain::{EntityType, EntityTypeAction},
    error::AppError,
};
use uuid::Uuid;

impl Store {
    pub(crate) async fn entity_types(
        &self,
        env: i64,
        deleted: bool,
    ) -> Result<Vec<EntityType>, AppError> {
        Ok(self
            .list::<EntityType>("entity_types", env)
            .await?
            .into_iter()
            .filter(|item| deleted || !item.is_deleted)
            .collect())
    }

    pub(crate) async fn create_entity_type(
        &self,
        env: i64,
        key: &str,
        title: &str,
        description: &str,
        actor: &str,
    ) -> Result<EntityType, AppError> {
        let key = crate::domain::validate_key(key, "entity type key")?;
        if self
            .entity_types(env, true)
            .await?
            .iter()
            .any(|item| item.type_key == key)
        {
            return Err(AppError::Conflict("entity type key already exists".into()));
        }
        let item = EntityType {
            id: Uuid::new_v4(),
            env_num: env,
            type_key: key,
            name: name(title)?,
            description: description.trim().into(),
            is_system: false,
            revision: 1,
            is_deleted: false,
        };
        self.put("entity_types", env, item.id, &item).await?;
        self.ensure_intrinsic_attributes(env, item.id, actor)
            .await?;
        self.audit(env, actor, "entity_type.create", item.id, 1)
            .await?;
        Ok(item)
    }

    pub(crate) async fn update_entity_type(
        &self,
        env: i64,
        id: Uuid,
        input: crate::database::model::inputs::DefinitionUpdate<'_>,
        actor: &str,
    ) -> Result<(), AppError> {
        let crate::database::model::inputs::DefinitionUpdate {
            title,
            description,
            deleted,
            expected,
        } = input;
        let mut item: EntityType = self.get("entity_types", env, id).await?;
        check_revision(item.revision, expected)?;
        if item.is_system && deleted {
            return Err(AppError::invalid("system entity type cannot be deleted"));
        }
        item.name = name(title)?;
        item.description = description.trim().into();
        item.is_deleted = deleted;
        item.revision += 1;
        self.put("entity_types", env, id, &item).await?;
        self.audit(env, actor, "entity_type.update", id, item.revision)
            .await
    }

    pub(crate) async fn actions(
        &self,
        env: i64,
        kind: Uuid,
        deleted: bool,
    ) -> Result<Vec<EntityTypeAction>, AppError> {
        self.get::<EntityType>("entity_types", env, kind).await?;
        Ok(self
            .list::<EntityTypeAction>("entity_type_actions", env)
            .await?
            .into_iter()
            .filter(|item| item.entity_type_id == kind && (deleted || !item.is_deleted))
            .collect())
    }

    pub(crate) async fn create_action(
        &self,
        env: i64,
        kind: Uuid,
        operation_type: &str,
        operation: &str,
        description: &str,
        actor: &str,
    ) -> Result<EntityTypeAction, AppError> {
        let entity_type: EntityType = self.get("entity_types", env, kind).await?;
        if entity_type.is_deleted {
            return Err(AppError::invalid("entity type is inactive"));
        }
        if !matches!(operation_type, "read" | "write") {
            return Err(AppError::invalid("operation_type must be read or write"));
        }
        let operation = name(operation)?;
        if self
            .actions(env, kind, true)
            .await?
            .iter()
            .any(|item| item.operation == operation && item.operation_type == operation_type)
        {
            return Err(AppError::Conflict("action already exists".into()));
        }
        let item = EntityTypeAction {
            id: numeric_id(),
            env_num: env,
            entity_type_id: kind,
            operation_type: operation_type.into(),
            operation,
            description: description.trim().into(),
            revision: 1,
            is_deleted: false,
        };
        self.put("entity_type_actions", env, item.id, &item).await?;
        self.audit(env, actor, "action.create", item.id, 1).await?;
        Ok(item)
    }

    pub(crate) async fn update_action(
        &self,
        env: i64,
        id: i64,
        description: &str,
        deleted: bool,
        expected: i64,
        actor: &str,
    ) -> Result<EntityTypeAction, AppError> {
        let mut item: EntityTypeAction = self.get("entity_type_actions", env, id).await?;
        check_revision(item.revision, expected)?;
        item.description = description.trim().into();
        item.is_deleted = deleted;
        item.revision += 1;
        self.put("entity_type_actions", env, id, &item).await?;
        self.audit(env, actor, "action.update", id, item.revision)
            .await?;
        Ok(item)
    }
}
