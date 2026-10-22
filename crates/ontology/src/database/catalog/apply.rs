use super::{
    directory_id, entity_id, type_id, validate, ACTOR, METADATA_ATTRIBUTE, NOTES_ATTRIBUTE,
    SOURCE_ATTRIBUTE,
};
use crate::{
    database::{model::inputs::TextWrite, text_revision::TextContent, Store},
    error::AppError,
    system_catalog::MEMBERSHIP_ID,
    text_store::TextStore,
};
use opencoder_core::ontology::Capability;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use uuid::Uuid;

impl Store {
    pub(crate) async fn catalog_apply(
        &self,
        env: i64,
        env_key: &str,
        entries: &[Capability],
        files: &TextStore,
    ) -> Result<Value, AppError> {
        validate(entries)?;
        let changes = self.catalog_check(env, entries).await?;
        if changes["in_sync"] == true {
            let previous = self.catalog_status(env).await?;
            return self
                .catalog_finish(env, &changes, previous["issues"].clone())
                .await;
        }
        self.catalog_initialize(env).await?;
        let mut desired_relations = BTreeSet::new();
        // Keep the managed directory's own parent edge.
        desired_relations.insert(
            self.catalog_relation(
                env,
                MEMBERSHIP_ID,
                directory_id(),
                crate::system_catalog::ROOT_DIRECTORY_ID,
                "平台能力目录",
                false,
            )
            .await?,
        );
        let mut issues = Vec::new();
        for entry in entries {
            let id = entity_id(&entry.key);
            self.catalog_put(
                "entities",
                env,
                id,
                json!({
                    "id":id,"env_num":env,"entity_type_id":type_id(),"name":entry.name,
                    "description":entry.description,"is_deleted":false
                }),
            )
            .await?;
            self.catalog_put("structured_values", env, format!("{id}/{METADATA_ATTRIBUTE}"), json!({
                "entity_id":id,"attribute_definition_id":METADATA_ATTRIBUTE.to_string(),
                "kind":"json","value":entry,"is_deleted":false,"created_by":ACTOR,"updated_by":ACTOR
            })).await?;
            self.put(
                "catalog_entries",
                env,
                id,
                &json!({"id":id,"capability":entry,"is_deleted":false}),
            )
            .await?;
            self.catalog_text(
                env,
                env_key,
                id,
                SOURCE_ATTRIBUTE,
                "# 平台能力来源\n\n由当前 Server 同步。来源标识、版本、依赖与原模块操作入口见普通属性中的“能力定义与入口”。\n",
                files,
            )
            .await?;
            self.catalog_text(
                env,
                env_key,
                id,
                NOTES_ATTRIBUTE,
                "# 补充说明\n\n可在此维护人工说明。\n",
                files,
            )
            .await?;
            desired_relations.insert(
                self.catalog_relation(
                    env,
                    MEMBERSHIP_ID,
                    id,
                    directory_id(),
                    "平台能力归属",
                    false,
                )
                .await?,
            );
            for dependency in &entry.dependencies {
                if entries.iter().any(|candidate| candidate.key == *dependency) {
                    desired_relations.insert(
                        self.catalog_relation(
                            env,
                            Uuid::from_u128(MEMBERSHIP_ID.as_u128() + 2),
                            id,
                            entity_id(dependency),
                            "定义声明的依赖",
                            false,
                        )
                        .await?,
                    );
                } else {
                    issues.push(json!({"key":entry.key,"missing_dependency":dependency}));
                }
            }
            for issue in &entry.issues {
                issues.push(json!({"key":entry.key,"error":issue}));
            }
        }
        for mut row in self.list::<Value>("catalog_entries", env).await? {
            if row["is_deleted"] == true
                || entries
                    .iter()
                    .any(|entry| row["capability"]["key"] == entry.key)
            {
                continue;
            }
            let id = row["id"]
                .as_str()
                .ok_or_else(|| AppError::dependency("catalog entry missing id"))?
                .to_owned();
            row["is_deleted"] = json!(true);
            self.put("catalog_entries", env, &id, &row).await?;
            let mut entity: Value = self.get("entities", env, &id).await?;
            entity["is_deleted"] = json!(true);
            self.catalog_put("entities", env, id, entity).await?;
        }
        for row in self.list::<Value>("catalog_objects", env).await? {
            if row["table"] != "relationships" {
                continue;
            }
            let id: Uuid = serde_json::from_value(row["id"].clone())
                .map_err(|_| AppError::dependency("invalid relation id"))?;
            if desired_relations.contains(&id) {
                continue;
            }
            let mut relation: Value = self.get("relationships", env, id).await?;
            relation["is_deleted"] = json!(true);
            self.catalog_put("relationships", env, id, relation).await?;
        }
        self.catalog_finish(env, &changes, json!(issues)).await
    }

    async fn catalog_finish(
        &self,
        env: i64,
        changes: &Value,
        issues: Value,
    ) -> Result<Value, AppError> {
        let status = json!({"state":"ready","last_success":chrono::Utc::now().to_rfc3339(),
            "last_attempt":chrono::Utc::now().to_rfc3339(),"error":null,"issues":issues,
            "created":changes["created"].as_array().map_or(0, Vec::len),
            "updated":changes["updated"].as_array().map_or(0, Vec::len),
            "deleted":changes["deleted"].as_array().map_or(0, Vec::len),
            "repaired":changes["drifted"].as_array().map_or(0, Vec::len)});
        self.put("catalog_state", env, "status", &status).await?;
        Ok(status)
    }

    async fn catalog_text(
        &self,
        env: i64,
        env_key: &str,
        entity: Uuid,
        attribute: i64,
        content: &str,
        files: &TextStore,
    ) -> Result<(), AppError> {
        let history = self.text_revisions(env, entity, attribute).await?;
        let current = history
            .iter()
            .find(|row| row["is_current"] == true && row["status"] == "ready");
        // Text is initialized once. Changing definitions live in the structured
        // attribute; immutable files remain safe to reuse after a rolled-back batch.
        if current.is_some() {
            return Ok(());
        }
        let revision = current
            .and_then(|row| row["revision"].as_i64())
            .unwrap_or(0);
        self.save_text_revision(
            files,
            TextWrite {
                env,
                env_key,
                entity,
                attribute,
                input: TextContent::Inline {
                    content: content.as_bytes(),
                    format: "md",
                },
                required: true,
                expected: revision,
            },
            ACTOR,
        )
        .await?;
        Ok(())
    }
}
