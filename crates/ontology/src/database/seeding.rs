use super::Store;
use crate::{
    domain::{Entity, EntityType, RelationshipType},
    error::AppError,
    system_catalog::*,
};

impl Store {
    pub(super) async fn initialize_debug(&self) -> Result<(), AppError> {
        match self.environment("debug").await {
            Ok(_) => Ok(()),
            Err(AppError::NotFound) => self
                .create_environment("debug", "Debug", "", "system")
                .await
                .map(|_| ()),
            Err(error) => Err(error),
        }
    }

    pub(super) async fn seed_environment(&self, env: i64, actor: &str) -> Result<(), AppError> {
        let kind = EntityType {
            id: DIRECTORY_TYPE_ID,
            env_num: env,
            type_key: "directory".into(),
            name: "目录".into(),
            description: "系统目录实体类型".into(),
            is_system: true,
            revision: 1,
            is_deleted: false,
        };
        self.put("entity_types", env, kind.id, &kind).await?;
        self.ensure_intrinsic_attributes(env, kind.id, actor)
            .await?;
        let root = Entity {
            id: ROOT_DIRECTORY_ID,
            env_num: env,
            entity_type_id: kind.id,
            name: "根目录".into(),
            description: "".into(),
            revision: 1,
            is_deleted: false,
        };
        self.put("entities", env, root.id, &root).await?;
        for (index, key, title) in [
            (0, "belongs_to", "属于"),
            (1, "contains", "包含"),
            (2, "depends_on", "依赖"),
            (3, "strong_depends_on", "强依赖"),
            (4, "weak_depends_on", "弱依赖"),
            (5, "text_description", "文本描述"),
        ] {
            let relation = RelationshipType {
                id: uuid::Uuid::from_u128(MEMBERSHIP_ID.as_u128() + index),
                env_num: env,
                type_key: key.into(),
                name: title.into(),
                description: "".into(),
                is_directory_membership: index == 0,
                is_system: true,
                source_entity_type_id: None,
                target_entity_type_ids: vec![],
                revision: 1,
                is_deleted: false,
            };
            self.put("relationship_types", env, relation.id, &relation)
                .await?;
        }
        Ok(())
    }
}
