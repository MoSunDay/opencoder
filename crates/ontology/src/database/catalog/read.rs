use super::{entity_id, validate, METADATA_ATTRIBUTE};
use crate::{database::Store, error::AppError};
use opencoder_core::ontology::Capability;
use serde_json::{json, Value};

impl Store {
    pub(crate) async fn catalog_start(&self, env: i64) -> Result<Value, AppError> {
        let mut status = self.catalog_status(env).await?;
        status["state"] = json!("running");
        status["last_attempt"] = json!(chrono::Utc::now().to_rfc3339());
        self.put("catalog_state", env, "status", &status).await?;
        Ok(status)
    }

    pub(crate) async fn catalog_list(&self, env: i64) -> Result<Value, AppError> {
        Ok(json!({"items":self.list::<Value>("catalog_entries", env).await?}))
    }

    pub(crate) async fn catalog_get(&self, env: i64, id: &str) -> Result<Value, AppError> {
        let id = id
            .parse::<uuid::Uuid>()
            .map_err(|_| AppError::invalid("invalid capability entity id"))?
            .to_string();
        let item: Value = self.get("catalog_entries", env, &id).await?;
        let management = self.catalog_management(env, "entities", &id).await?;
        Ok(json!({"item":item,"management":management}))
    }

    pub(crate) async fn catalog_management(
        &self,
        env: i64,
        table: &str,
        id: &str,
    ) -> Result<Value, AppError> {
        let row = self
            .optional::<Value>("catalog_objects", env, format!("{table}:{id}"))
            .await?;
        Ok(match row {
            None => Value::Null,
            Some(_) => {
                let entry = self.optional::<Value>("catalog_entries", env, id).await?;
                json!({"managed_by":"platform-catalog", "readonly_attributes":[METADATA_ATTRIBUTE.to_string(), super::SOURCE_ATTRIBUTE.to_string()],
                    "cli":entry.as_ref().map(|v| &v["capability"]["cli"]),
                    "api":entry.as_ref().map(|v| &v["capability"]["api"])})
            }
        })
    }

    pub(crate) async fn catalog_status(&self, env: i64) -> Result<Value, AppError> {
        Ok(self
            .optional::<Value>("catalog_state", env, "status")
            .await?
            .unwrap_or(json!({"state":"not_initialized","last_success":null,"error":null})))
    }

    pub(crate) async fn catalog_error(&self, env: i64, error: &str) -> Result<Value, AppError> {
        let mut status = self.catalog_status(env).await?;
        status["state"] = json!("error");
        status["error"] = json!(error);
        status["last_attempt"] = json!(chrono::Utc::now().to_rfc3339());
        self.put("catalog_state", env, "status", &status).await?;
        Ok(status)
    }

    pub(crate) async fn catalog_check(
        &self,
        env: i64,
        entries: &[Capability],
    ) -> Result<Value, AppError> {
        validate(entries)?;
        let existing = self.list::<Value>("catalog_entries", env).await?;
        let mut created = Vec::new();
        let mut updated = Vec::new();
        for entry in entries {
            let id = entity_id(&entry.key).to_string();
            match existing.iter().find(|value| value["id"] == id) {
                None => created.push(entry.key.clone()),
                Some(value)
                    if value["is_deleted"] == true || value["capability"] != json!(entry) =>
                {
                    updated.push(entry.key.clone())
                }
                _ => {}
            }
        }
        let deleted: Vec<_> = existing
            .iter()
            .filter(|value| value["is_deleted"] != true)
            .filter(|value| {
                !entries
                    .iter()
                    .any(|entry| value["capability"]["key"] == entry.key)
            })
            .map(|value| value["capability"]["key"].clone())
            .collect();
        let initialized = self.catalog_status(env).await?["last_success"].is_string();
        let mut drifted = Vec::new();
        for object in self.list::<Value>("catalog_objects", env).await? {
            let table = object["table"]
                .as_str()
                .ok_or_else(|| AppError::dependency("invalid catalog ownership"))?;
            if !matches!(
                table,
                "entities"
                    | "entity_types"
                    | "attribute_definitions"
                    | "structured_values"
                    | "relationships"
                    | "graph_aspects"
            ) {
                return Err(AppError::dependency("invalid catalog object table"));
            }
            let id = object["id"]
                .as_str()
                .ok_or_else(|| AppError::dependency("invalid catalog object id"))?;
            let current = self.optional::<Value>(table, env, id).await?;
            if current.as_ref().is_none_or(|current| {
                json!(super::super::store::digest_id(&current.to_string())) != object["fingerprint"]
            }) {
                drifted.push(format!("{table}:{id}"));
            }
        }
        Ok(
            json!({"in_sync":initialized && created.is_empty() && updated.is_empty() && deleted.is_empty() && drifted.is_empty(),
            "created":created,"updated":updated,"deleted":deleted,"drifted":drifted}),
        )
    }
}
