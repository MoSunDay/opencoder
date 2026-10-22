use super::ACTOR;
use crate::{database::Store, error::AppError};
use serde_json::{json, Value};

impl Store {
    pub(crate) async fn catalog_guard(&self, path: &str) -> Result<(), AppError> {
        let parts: Vec<String> = path
            .split('/')
            .map(|part| {
                let encoded = format!("v={}", part.replace('+', "%2B"));
                url::form_urlencoded::parse(encoded.as_bytes())
                    .next()
                    .map(|(_, value)| value.into_owned())
                    .unwrap_or_default()
            })
            .collect();
        let Some(index) = parts.iter().position(|part| part == "envs") else {
            return Ok(());
        };
        let parts = &parts[index + 1..];
        if parts.len() < 3 {
            return Ok(());
        }
        let env = self.environment(&parts[0]).await?.env_num;
        let (table, id) = match parts[1].as_str() {
            "entities" if parts.len() >= 5 && parts[3] == "attributes" => {
                if parts[4].parse::<i64>().ok() == Some(super::NOTES_ATTRIBUTE) {
                    return Ok(());
                }
                ("attribute_definitions", parts[4].as_str())
            }
            "entities" if parts.len() == 3 => ("entities", parts[2].as_str()),
            "directories" if parts[2] != "tree" => ("entities", parts[2].as_str()),
            "entity-types" if parts.len() == 3 => ("entity_types", parts[2].as_str()),
            "attributes" => ("attribute_definitions", parts[2].as_str()),
            "relationships" => ("relationships", parts[2].as_str()),
            "graph-aspects" => ("graph_aspects", parts[2].as_str()),
            _ => return Ok(()),
        };
        let canonical = if table == "attribute_definitions" {
            id.parse::<i64>()
                .map(|id| id.to_string())
                .unwrap_or_else(|_| id.into())
        } else {
            id.parse::<uuid::Uuid>()
                .map(|id| id.to_string())
                .unwrap_or_else(|_| id.into())
        };
        self.catalog_assert_editable(env, table, &canonical).await
    }

    pub(super) async fn catalog_put(
        &self,
        table: &str,
        env: i64,
        id: impl ToString,
        mut value: Value,
    ) -> Result<(), AppError> {
        let id = id.to_string();
        let key = format!("{table}:{id}");
        let previous = self.optional::<Value>(table, env, &id).await?;
        if previous.is_some()
            && self
                .optional::<Value>("catalog_objects", env, &key)
                .await?
                .is_none()
        {
            return Err(AppError::Conflict(format!(
                "catalog object already exists outside platform ownership: {table}/{id}"
            )));
        }
        let revision = previous
            .as_ref()
            .and_then(|v| v["revision"].as_i64())
            .unwrap_or(0);
        value["revision"] = json!(revision);
        let changed = previous.as_ref() != Some(&value);
        if changed {
            value["revision"] = json!(revision + 1);
            self.put(table, env, &id, &value).await?;
        }
        let fingerprint = super::super::store::digest_id(&value.to_string());
        self.put(
            "catalog_objects",
            env,
            key,
            &json!({"table":table,"id":id,"fingerprint":fingerprint}),
        )
        .await?;
        if changed {
            self.audit(env, ACTOR, &format!("catalog.{table}"), id, revision + 1)
                .await?;
        }
        Ok(())
    }

    pub(crate) async fn catalog_assert_editable(
        &self,
        env: i64,
        table: &str,
        id: &str,
    ) -> Result<(), AppError> {
        let management = self.catalog_management(env, table, id).await?;
        if !management.is_null() {
            return Err(AppError::Conflict(format!(
                "由平台同步维护，请通过原模块入口修改；ontology capabilities get 可查询来源。{management}"
            )));
        }
        Ok(())
    }
}
