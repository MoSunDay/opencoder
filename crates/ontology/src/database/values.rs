use super::{model::policy::nonempty, store::check_revision, Store};
use crate::{
    domain::{AttributeKind, AttributeRole, Entity},
    error::AppError,
    system_catalog::DIRECTORY_TYPE_ID,
};
use serde_json::{json, Value};
use uuid::Uuid;

impl Store {
    pub(crate) async fn set_structured_value(
        &self,
        env: i64,
        entity: Uuid,
        attribute: i64,
        input: crate::database::model::inputs::StructuredWrite<'_>,
        actor: &str,
    ) -> Result<Value, AppError> {
        let crate::database::model::inputs::StructuredWrite {
            kind,
            value,
            deleted,
            expected,
        } = input;
        validate_structured_value(&kind, value)?;
        self.entity(env, entity).await?;
        let id = format!("{entity}/{attribute}");
        let current = self
            .optional::<Value>("structured_values", env, &id)
            .await?;
        let actual = current
            .as_ref()
            .and_then(|item| item["revision"].as_i64())
            .unwrap_or(0);
        check_revision(actual, expected)?;
        let now = chrono::Utc::now().to_rfc3339();
        let created_by = current
            .as_ref()
            .map(|item| item["created_by"].clone())
            .unwrap_or(json!(actor));
        let created_at = current
            .as_ref()
            .map(|item| item["created_at"].clone())
            .unwrap_or(json!(now));
        let item = json!({"entity_id":entity,"attribute_definition_id":attribute.to_string(),"kind":kind,"value":value,"is_deleted":deleted,"revision":actual+1,"created_by":created_by,"created_at":created_at,"updated_by":actor,"updated_at":now});
        self.put("structured_values", env, &id, &item).await?;
        self.audit(env, actor, "attribute.value", id, actual + 1)
            .await?;
        Ok(item)
    }

    pub(crate) async fn structured_values(
        &self,
        env: i64,
        entity: Uuid,
    ) -> Result<Vec<Value>, AppError> {
        Ok(self
            .list::<Value>("structured_values", env)
            .await?
            .into_iter()
            .filter(|item| item["entity_id"] == entity.to_string())
            .collect())
    }

    pub(crate) async fn validate_required_values(
        &self,
        env: i64,
        entity: &Entity,
    ) -> Result<(), AppError> {
        if entity.entity_type_id == DIRECTORY_TYPE_ID {
            return Ok(());
        }
        let definitions = self
            .attributes(env, Some(entity.entity_type_id), false)
            .await?;
        let values = self.structured_values(env, entity.id).await?;
        for definition in definitions.iter().filter(|item| item.required) {
            let complete = if definition.kind == AttributeKind::Text {
                self.text_revisions(env, entity.id, definition.id)
                    .await?
                    .iter()
                    .any(|item| {
                        item["is_current"] == true
                            && item["status"] == "ready"
                            && item["bytes"].as_u64().is_some_and(|bytes| bytes > 0)
                    })
            } else {
                values.iter().any(|item| {
                    item["attribute_definition_id"]
                        .as_str()
                        .and_then(|id| id.parse::<i64>().ok())
                        == Some(definition.id)
                        && item["is_deleted"] == false
                        && nonempty(&item["value"])
                })
            };
            if !complete {
                return Err(AppError::invalid(format!(
                    "required attribute {} is missing",
                    definition.attribute_key
                )));
            }
        }
        Ok(())
    }

    pub(crate) async fn intrinsic_complete(
        &self,
        env: i64,
        entity: Uuid,
    ) -> Result<bool, AppError> {
        let item = self.entity(env, entity).await?;
        if item.entity_type_id == DIRECTORY_TYPE_ID {
            return Ok(true);
        }
        for definition in self
            .attributes(env, Some(item.entity_type_id), false)
            .await?
            .iter()
            .filter(|item| item.attribute_role != AttributeRole::Custom)
        {
            if !self
                .text_revisions(env, entity, definition.id)
                .await?
                .iter()
                .any(|item| {
                    item["is_current"] == true
                        && item["bytes"].as_u64().is_some_and(|bytes| bytes > 0)
                })
            {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

pub(crate) fn validate_structured_value(
    kind: &AttributeKind,
    value: &Value,
) -> Result<(), AppError> {
    let valid = match kind {
        AttributeKind::String => value.is_string(),
        AttributeKind::Integer => value.as_i64().is_some(),
        AttributeKind::Float => value.as_f64().is_some_and(f64::is_finite),
        AttributeKind::Boolean => value.is_boolean(),
        AttributeKind::Datetime => value
            .as_str()
            .is_some_and(|text| chrono::DateTime::parse_from_rfc3339(text).is_ok()),
        AttributeKind::Json => true,
        AttributeKind::Vector | AttributeKind::Text => false,
    };
    if valid {
        Ok(())
    } else {
        Err(AppError::invalid("attribute value does not match its kind"))
    }
}
