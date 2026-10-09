use crate::{
    domain::{AttributeKind, AttributeRole, StorageMode},
    error::AppError,
};
use serde_json::Value;
use uuid::Uuid;

pub(crate) fn validate_attribute(
    kind: &AttributeKind,
    role: &AttributeRole,
    storage: &StorageMode,
    required: bool,
) -> Result<(), AppError> {
    if *role != AttributeRole::Custom && (!required || *kind != AttributeKind::Text) {
        return Err(AppError::invalid(
            "source/ext must be required text attributes",
        ));
    }
    let valid = match role {
        AttributeRole::Source => *storage == StorageMode::Markdown,
        AttributeRole::Ext => matches!(storage, StorageMode::Markdown | StorageMode::NfsPath),
        AttributeRole::Custom => {
            if *kind == AttributeKind::Text {
                *storage == StorageMode::Markdown
            } else {
                *storage == StorageMode::Sql
            }
        }
    };
    if !valid {
        return Err(AppError::invalid(
            "attribute storage mode does not match its role/kind",
        ));
    }
    Ok(())
}

pub(crate) fn nonempty(value: &Value) -> bool {
    !value.is_null() && value.as_str().is_none_or(|s| !s.trim().is_empty())
}

pub(crate) fn validate_scope(
    source: Option<Uuid>,
    targets: &[Uuid],
    legacy: bool,
) -> Result<(), AppError> {
    if source.is_none() && !legacy {
        return Err(AppError::invalid(
            "relationship types require a source entity type",
        ));
    }
    if source.is_some() && targets.is_empty() {
        return Err(AppError::invalid(
            "relationship template requires at least one target type",
        ));
    }
    if source.is_none() && !targets.is_empty() {
        return Err(AppError::invalid(
            "target entity types require a source entity type",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn intrinsic_storage_and_required_rules() {
        assert!(validate_attribute(
            &AttributeKind::Text,
            &AttributeRole::Source,
            &StorageMode::Markdown,
            true
        )
        .is_ok());
        assert!(validate_attribute(
            &AttributeKind::Text,
            &AttributeRole::Ext,
            &StorageMode::NfsPath,
            true
        )
        .is_ok());
        assert!(validate_attribute(
            &AttributeKind::Text,
            &AttributeRole::Ext,
            &StorageMode::Sql,
            true
        )
        .is_err());
        assert!(validate_attribute(
            &AttributeKind::String,
            &AttributeRole::Source,
            &StorageMode::Markdown,
            true
        )
        .is_err());
        assert!(validate_attribute(
            &AttributeKind::Text,
            &AttributeRole::Source,
            &StorageMode::Markdown,
            false
        )
        .is_err());
        assert!(validate_attribute(
            &AttributeKind::Text,
            &AttributeRole::Custom,
            &StorageMode::NfsPath,
            false
        )
        .is_err());
    }
    #[test]
    fn scoped_relations_need_source_and_targets() {
        let id = Uuid::new_v4();
        assert!(validate_scope(None, &[], true).is_ok());
        assert!(validate_scope(None, &[], false).is_err());
        assert!(validate_scope(Some(id), &[], false).is_err());
        assert!(validate_scope(Some(id), &[id], false).is_ok());
    }
    #[test]
    fn required_false_and_zero_are_values() {
        assert!(nonempty(&serde_json::json!(false)));
        assert!(nonempty(&serde_json::json!(0)));
        assert!(!nonempty(&serde_json::json!("  ")));
        assert!(!nonempty(&Value::Null));
    }
}
