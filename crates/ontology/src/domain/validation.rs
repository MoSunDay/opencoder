use super::types::*;
use crate::error::AppError;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

pub fn validate_key(value: &str, label: &str) -> Result<String, AppError> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty()
        || normalized.len() > 64
        || !normalized
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-'))
    {
        return Err(AppError::invalid(format!(
            "{label} must be a lowercase identifier"
        )));
    }
    Ok(normalized)
}

pub fn validate_aspect(
    aspect_key: &str,
    name: &str,
    description: &str,
    entity_type_ids: Vec<Uuid>,
    relationship_type_ids: Vec<Uuid>,
    defaults: AspectDefaults,
) -> Result<AspectPatch, AppError> {
    let aspect_key = validate_key(aspect_key, "aspect key")?;
    let name = name.trim();
    if name.is_empty() || name.len() > 255 {
        return Err(AppError::invalid(
            "aspect name must contain 1 to 255 characters",
        ));
    }
    let description = description.trim();
    if description.len() > 1024 {
        return Err(AppError::invalid(
            "aspect description must contain at most 1024 characters",
        ));
    }
    let entity_type_ids = dedup_uuids(entity_type_ids);
    let relationship_type_ids = dedup_uuids(relationship_type_ids);
    if entity_type_ids.is_empty() {
        return Err(AppError::invalid(
            "a graph aspect must pin at least one entity type",
        ));
    }
    if defaults.default_upstream_depth.is_some() != defaults.default_downstream_depth.is_some()
        || defaults
            .default_upstream_depth
            .is_some_and(|depth| depth > 9)
        || defaults
            .default_downstream_depth
            .is_some_and(|depth| depth > 9)
    {
        return Err(AppError::invalid(
            "aspect default depths must both be present and between 0 and 9",
        ));
    }
    let mut defaults = defaults;
    defaults.default_center_ids = dedup_uuids(defaults.default_center_ids);
    Ok(AspectPatch {
        aspect_key,
        name: name.into(),
        description: description.into(),
        entity_type_ids,
        relationship_type_ids,
        defaults,
    })
}

fn dedup_uuids(ids: Vec<Uuid>) -> Vec<Uuid> {
    let mut seen = HashSet::new();
    ids.into_iter().filter(|id| seen.insert(*id)).collect()
}

pub fn validate_vector(values: &[f32]) -> Result<(), AppError> {
    if values.len() != VECTOR_DIMENSION {
        return Err(AppError::invalid("vector must contain exactly 2048 values"));
    }
    if values.iter().any(|v| !v.is_finite()) {
        return Err(AppError::invalid("vector values must be finite"));
    }
    if values.iter().all(|v| *v == 0.0) {
        return Err(AppError::invalid("vector must not be all zero"));
    }
    Ok(())
}

pub fn validate_directory_move<S: std::hash::BuildHasher>(
    entity_id: Uuid,
    parent_id: Uuid,
    parents: &HashMap<Uuid, Uuid, S>,
) -> Result<(), AppError> {
    if entity_id == parent_id {
        return Err(AppError::invalid("a directory cannot contain itself"));
    }
    let mut current = parent_id;
    let mut visited = HashSet::new();
    while let Some(parent) = parents.get(&current) {
        if current == entity_id || *parent == entity_id {
            return Err(AppError::invalid("directory move would create a cycle"));
        }
        if !visited.insert(current) {
            return Err(AppError::dependency(
                "existing directory graph contains a cycle",
            ));
        }
        current = *parent;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vector_contract_rejects_wrong_zero_and_non_finite_values() {
        assert!(validate_vector(&[1.0]).is_err());
        assert!(validate_vector(&vec![0.0; VECTOR_DIMENSION]).is_err());
        let mut invalid = vec![1.0; VECTOR_DIMENSION];
        invalid[7] = f32::NAN;
        assert!(validate_vector(&invalid).is_err());
        assert!(validate_vector(&vec![1.0; VECTOR_DIMENSION]).is_ok());
    }

    #[test]
    fn directory_move_rejects_cycles() {
        let root = Uuid::new_v4();
        let child = Uuid::new_v4();
        let leaf = Uuid::new_v4();
        let parents = HashMap::from([(child, root), (leaf, child)]);
        assert!(validate_directory_move(root, leaf, &parents).is_err());
        assert!(validate_directory_move(leaf, root, &parents).is_ok());
    }

    #[test]
    fn validate_aspect_normalizes_key_and_dedups_ids() {
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let patch = validate_aspect(
            " Ops-Overview ",
            "  运营总览  ",
            "  只看运营相关节点 ",
            vec![first, second, first],
            vec![second, second],
            AspectDefaults::default(),
        )
        .unwrap();
        assert_eq!(patch.aspect_key, "ops-overview");
        assert_eq!(patch.name, "运营总览");
        assert_eq!(patch.description, "只看运营相关节点");
        assert_eq!(patch.entity_type_ids, vec![first, second]);
        assert_eq!(patch.relationship_type_ids, vec![second]);
    }

    #[test]
    fn validate_aspect_rejects_bad_key_blank_name_and_long_description() {
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        assert!(validate_aspect(
            "Bad Key",
            "name",
            "",
            vec![first],
            vec![second],
            AspectDefaults::default()
        )
        .is_err());
        assert!(validate_aspect(
            "",
            "name",
            "",
            vec![first],
            vec![second],
            AspectDefaults::default()
        )
        .is_err());
        assert!(validate_aspect(
            "ok",
            "   ",
            "",
            vec![first],
            vec![second],
            AspectDefaults::default()
        )
        .is_err());
        assert!(
            validate_aspect(
                "ok",
                &"x".repeat(256),
                "",
                vec![first],
                vec![second],
                AspectDefaults::default()
            )
            .is_err(),
            "name longer than 255 characters must be rejected"
        );
        assert!(
            validate_aspect(
                "ok",
                "name",
                &"x".repeat(1025),
                vec![first],
                vec![second],
                AspectDefaults::default()
            )
            .is_err(),
            "description longer than 1024 characters must be rejected"
        );
    }

    #[test]
    fn validate_aspect_requires_entity_types_but_allows_all_relationships() {
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        assert!(
            validate_aspect("ok", "name", "", vec![], vec![], AspectDefaults::default()).is_err()
        );
        assert!(validate_aspect(
            "ok",
            "name",
            "",
            vec![first],
            vec![],
            AspectDefaults::default()
        )
        .is_ok());
        assert!(validate_aspect(
            "ok",
            "name",
            "",
            vec![],
            vec![second],
            AspectDefaults::default()
        )
        .is_err());
        assert!(validate_aspect(
            "ok",
            "name",
            "",
            vec![first],
            vec![second],
            AspectDefaults::default()
        )
        .is_ok());
    }

    #[test]
    fn validate_aspect_defaults_are_bounded_and_deduplicated() {
        let center = Uuid::new_v4();
        let defaults = AspectDefaults {
            default_center_ids: vec![center, center],
            default_upstream_depth: Some(2),
            default_downstream_depth: Some(0),
        };
        let patch = validate_aspect("ok", "name", "", vec![center], vec![], defaults).unwrap();
        assert_eq!(patch.defaults.default_center_ids, vec![center]);
        assert_eq!(patch.defaults.default_upstream_depth, Some(2));
        assert!(validate_aspect(
            "ok",
            "name",
            "",
            vec![center],
            vec![],
            AspectDefaults {
                default_upstream_depth: Some(1),
                ..AspectDefaults::default()
            }
        )
        .is_err());
        assert!(validate_aspect(
            "ok",
            "name",
            "",
            vec![center],
            vec![],
            AspectDefaults {
                default_upstream_depth: Some(10),
                default_downstream_depth: Some(1),
                ..AspectDefaults::default()
            }
        )
        .is_err());
    }

    #[test]
    fn graph_aspect_serializes_explicit_id_lists() {
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let aspect = GraphAspect {
            id: Uuid::new_v4(),
            env_num: 1,
            aspect_key: "ops".into(),
            name: "运营".into(),
            description: String::new(),
            entity_type_ids: vec![first],
            relationship_type_ids: vec![first, second],
            defaults: AspectDefaults {
                default_center_ids: vec![first],
                default_upstream_depth: Some(0),
                default_downstream_depth: Some(2),
            },
            revision: 3,
            is_deleted: false,
        };
        let raw = serde_json::to_string(&aspect).unwrap();
        assert!(raw.contains("\"entity_type_ids\":["));
        assert!(raw.contains("\"default_center_ids\":["));
        let parsed: GraphAspect = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed.entity_type_ids, aspect.entity_type_ids);
        assert_eq!(parsed.relationship_type_ids, aspect.relationship_type_ids);
        assert_eq!(
            parsed.defaults.default_center_ids,
            aspect.defaults.default_center_ids
        );
        assert_eq!(parsed.defaults.default_upstream_depth, Some(0));
        assert_eq!(parsed.revision, 3);
    }
}
