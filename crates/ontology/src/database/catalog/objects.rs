use super::{
    aspect_id, directory_id, type_id, METADATA_ATTRIBUTE, NOTES_ATTRIBUTE, SOURCE_ATTRIBUTE,
};
use crate::{database::Store, error::AppError, system_catalog::*};
use serde_json::json;

impl Store {
    pub(super) async fn catalog_initialize(&self, env: i64) -> Result<(), AppError> {
        let kind = type_id();
        self.catalog_put(
            "entity_types",
            env,
            kind,
            json!({
                "id":kind,"env_num":env,"type_key":"platform_capability","name":"平台能力",
                "description":"平台功能入口与可执行能力；原始定义由来源模块维护",
                "is_system":true,"is_deleted":false
            }),
        )
        .await?;
        for (id, key, name, value_kind, role, storage) in [
            (
                METADATA_ATTRIBUTE,
                "platform_definition",
                "能力定义与入口",
                "json",
                "custom",
                "sql",
            ),
            (
                SOURCE_ATTRIBUTE,
                "source",
                "来源",
                "text",
                "source",
                "markdown",
            ),
            (
                NOTES_ATTRIBUTE,
                "ext",
                "补充说明",
                "text",
                "ext",
                "markdown",
            ),
        ] {
            self.catalog_put(
                "attribute_definitions",
                env,
                id,
                json!({
                    "id":id.to_string(),"env_num":env,"entity_type_id":kind,"attribute_key":key,
                    "name":name,"description":"","kind":value_kind,"attribute_role":role,
                    "storage_mode":storage,"required":role!="custom","is_deleted":false
                }),
            )
            .await?;
        }
        self.catalog_put(
            "entities",
            env,
            directory_id(),
            json!({
                "id":directory_id(),"env_num":env,"entity_type_id":DIRECTORY_TYPE_ID,
                "name":"平台能力","description":"由当前 Server 自动维护","is_deleted":false
            }),
        )
        .await?;
        self.catalog_relation(
            env,
            MEMBERSHIP_ID,
            directory_id(),
            ROOT_DIRECTORY_ID,
            "平台能力目录",
            false,
        )
        .await?;
        self.catalog_put("graph_aspects", env, aspect_id(), json!({
            "id":aspect_id(),"env_num":env,"aspect_key":"platform_capabilities","name":"平台能力",
            "description":"平台入口、定义与明确声明的依赖","entity_type_ids":[kind],
            "relationship_type_ids":[MEMBERSHIP_ID, uuid::Uuid::from_u128(MEMBERSHIP_ID.as_u128()+2)],
            "default_center_ids":[],"default_upstream_depth":1,"default_downstream_depth":1,"is_deleted":false
        })).await
    }

    pub(super) async fn catalog_relation(
        &self,
        env: i64,
        kind: uuid::Uuid,
        source: uuid::Uuid,
        target: uuid::Uuid,
        description: &str,
        deleted: bool,
    ) -> Result<uuid::Uuid, AppError> {
        let id = super::super::store::digest_id(&format!(
            "platform-catalog/relation/{kind}/{source}/{target}"
        ));
        self.catalog_put("relationships", env, id, json!({
            "id":id,"env_num":env,"relationship_type_id":kind,"source_entity_id":source,
            "target_entity_id":target,"description":description,"is_deleted":deleted,"is_pinned":false
        })).await?;
        Ok(id)
    }
}
