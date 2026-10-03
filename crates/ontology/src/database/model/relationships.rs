use super::{
    super::{
        store::{check_revision, name},
        Store,
    },
    policy::validate_scope,
};
use crate::{
    domain::{EntityType, Relationship, RelationshipType},
    error::AppError,
    system_catalog::*,
};
use uuid::Uuid;

impl Store {
    pub(crate) async fn relationship_types(
        &self,
        env: i64,
        deleted: bool,
    ) -> Result<Vec<RelationshipType>, AppError> {
        Ok(self
            .list::<RelationshipType>("relationship_types", env)
            .await?
            .into_iter()
            .filter(|item| deleted || !item.is_deleted)
            .collect())
    }

    async fn validate_relation_scope(
        &self,
        env: i64,
        source: Option<Uuid>,
        targets: &[Uuid],
        system: bool,
    ) -> Result<(), AppError> {
        validate_scope(source, targets, system)?;
        for id in source.into_iter().chain(targets.iter().copied()) {
            let item: EntityType = self.get("entity_types", env, id).await?;
            if item.is_deleted {
                return Err(AppError::invalid(
                    "relationship template references an inactive entity type",
                ));
            }
        }
        Ok(())
    }

    pub(crate) async fn create_relationship_type_config(
        &self,
        env: i64,
        input: crate::database::model::inputs::RelationshipTypeCreate<'_>,
        actor: &str,
    ) -> Result<RelationshipType, AppError> {
        let crate::database::model::inputs::RelationshipTypeCreate {
            key,
            title,
            description,
            source,
            targets,
        } = input;
        let key = crate::domain::validate_key(key, "relationship type key")?;
        self.validate_relation_scope(env, source, targets, false)
            .await?;
        if self
            .relationship_types(env, true)
            .await?
            .iter()
            .any(|item| item.type_key == key)
        {
            return Err(AppError::Conflict(
                "relationship type key already exists".into(),
            ));
        }
        let mut targets = targets.to_vec();
        targets.sort();
        targets.dedup();
        let item = RelationshipType {
            id: Uuid::new_v4(),
            env_num: env,
            type_key: key,
            name: name(title)?,
            description: description.trim().into(),
            is_system: false,
            is_directory_membership: false,
            source_entity_type_id: source,
            target_entity_type_ids: targets,
            revision: 1,
            is_deleted: false,
        };
        self.put("relationship_types", env, item.id, &item).await?;
        self.audit(env, actor, "relationship_type.create", item.id, 1)
            .await?;
        Ok(item)
    }

    pub(crate) async fn update_relationship_type_with_scope(
        &self,
        env: i64,
        id: Uuid,
        input: crate::database::model::inputs::RelationshipTypeUpdate<'_>,
        actor: &str,
    ) -> Result<RelationshipType, AppError> {
        let crate::database::model::inputs::RelationshipTypeUpdate {
            title,
            description,
            source,
            targets,
            deleted,
            expected,
        } = input;
        let mut item: RelationshipType = self.get("relationship_types", env, id).await?;
        check_revision(item.revision, expected)?;
        if item.is_system
            && (deleted
                || source != item.source_entity_type_id
                || targets != item.target_entity_type_ids)
        {
            return Err(AppError::invalid(
                "system relationship type scope cannot change or be deleted",
            ));
        }
        self.validate_relation_scope(env, source, targets, item.is_system)
            .await?;
        item.name = name(title)?;
        item.description = description.trim().into();
        item.is_deleted = deleted;
        item.source_entity_type_id = source;
        item.target_entity_type_ids = targets.to_vec();
        item.target_entity_type_ids.sort();
        item.target_entity_type_ids.dedup();
        item.revision += 1;
        self.put("relationship_types", env, id, &item).await?;
        self.audit(env, actor, "relationship_type.update", id, item.revision)
            .await?;
        Ok(item)
    }

    pub(crate) async fn relationships(
        &self,
        env: i64,
        anchor: Option<Uuid>,
        deleted: bool,
    ) -> Result<Vec<Relationship>, AppError> {
        let sql = "SELECT body FROM relationships WHERE env_num=?1 AND (?2 IS NULL OR json_extract(body,'$.source_entity_id')=?2 OR json_extract(body,'$.target_entity_id')=?2) AND (?3 OR NOT json_extract(body,'$.is_deleted')) ORDER BY id";
        let mut rows = self
            .connection
            .query(
                sql,
                libsql::params![env, anchor.map(|id| id.to_string()), deleted as i64],
            )
            .await?;
        let mut items = Vec::new();
        while let Some(row) = rows.next().await? {
            items.push(super::super::store::decode(&row.get::<String>(0)?)?);
        }
        Ok(items)
    }

    pub(crate) async fn relationship(&self, env: i64, id: Uuid) -> Result<Relationship, AppError> {
        self.get("relationships", env, id).await
    }

    pub(crate) async fn create_relationship(
        &self,
        env: i64,
        kind: Uuid,
        source: Uuid,
        target: Uuid,
        description: &str,
        actor: &str,
    ) -> Result<Relationship, AppError> {
        if source == target {
            return Err(AppError::invalid("relationship endpoints must differ"));
        }
        let source_entity = self.entity(env, source).await?;
        let target_entity = self.entity(env, target).await?;
        let template: RelationshipType = self.get("relationship_types", env, kind).await?;
        if template.is_deleted || source_entity.is_deleted || target_entity.is_deleted {
            return Err(AppError::invalid(
                "relationship type and endpoints must be active",
            ));
        }
        if template
            .source_entity_type_id
            .is_some_and(|id| id != source_entity.entity_type_id)
            || (template.source_entity_type_id.is_some()
                && !template
                    .target_entity_type_ids
                    .contains(&target_entity.entity_type_id))
        {
            return Err(AppError::invalid(
                "relationship endpoints do not match template",
            ));
        }
        if template.is_directory_membership {
            if source == ROOT_DIRECTORY_ID || target_entity.entity_type_id != DIRECTORY_TYPE_ID {
                return Err(AppError::invalid("invalid directory membership"));
            }
            let relations = self.relationships(env, None, false).await?;
            if relations
                .iter()
                .any(|item| item.relationship_type_id == kind && item.source_entity_id == source)
            {
                return Err(AppError::Conflict(
                    "entity already belongs to a directory".into(),
                ));
            }
            let parents = relations
                .into_iter()
                .filter(|item| item.relationship_type_id == kind)
                .map(|item| (item.source_entity_id, item.target_entity_id))
                .collect::<std::collections::HashMap<_, _>>();
            crate::domain::validate_directory_move(source, target, &parents)?;
        }
        let item = Relationship {
            id: Uuid::new_v4(),
            env_num: env,
            relationship_type_id: kind,
            source_entity_id: source,
            target_entity_id: target,
            description: description.trim().into(),
            revision: 1,
            is_deleted: false,
            is_pinned: false,
        };
        self.put("relationships", env, item.id, &item).await?;
        self.audit(env, actor, "relationship.create", item.id, 1)
            .await?;
        Ok(item)
    }

    pub(crate) async fn update_relationship(
        &self,
        env: i64,
        id: Uuid,
        input: crate::database::model::inputs::RelationshipUpdate<'_>,
        actor: &str,
    ) -> Result<Relationship, AppError> {
        let crate::database::model::inputs::RelationshipUpdate {
            description,
            deleted,
            pinned,
            expected,
        } = input;
        let mut item = self.relationship(env, id).await?;
        check_revision(item.revision, expected)?;
        item.description = description.trim().into();
        item.is_deleted = deleted;
        item.is_pinned = !deleted && pinned.unwrap_or(item.is_pinned);
        item.revision += 1;
        self.put("relationships", env, id, &item).await?;
        self.audit(env, actor, "relationship.update", id, item.revision)
            .await?;
        Ok(item)
    }
}
