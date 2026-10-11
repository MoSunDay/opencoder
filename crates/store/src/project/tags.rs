//! Pure scope resolution shared by all project storage backends.
use crate::{ProjectInitiativeRecord, ProjectTodoRecord};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectTag {
    pub id: String,
    pub scope_type: String,
    pub scope_id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectTodoTag {
    pub todo_id: String,
    pub tag_id: String,
}

#[derive(Debug, thiserror::Error)]
pub enum TagError {
    #[error("tag name already exists in this scope")]
    Duplicate,
    #[error("tag is not available for this TODO")]
    InvalidScope,
}

fn in_scope(tag: &ProjectTag, initiative: Option<&ProjectInitiativeRecord>) -> bool {
    match tag.scope_type.as_str() {
        "global" => tag.scope_id.is_empty(),
        "project" => {
            initiative.is_some_and(|i| i.goal_id.as_deref() == Some(tag.scope_id.as_str()))
        }
        "initiative" => initiative.is_some_and(|i| i.id == tag.scope_id),
        _ => false,
    }
}

pub fn available_tags<'a>(
    tags: &'a [ProjectTag],
    initiative: Option<&ProjectInitiativeRecord>,
) -> Vec<&'a ProjectTag> {
    let mut names = BTreeMap::new();
    for scope in ["global", "project", "initiative"] {
        for tag in tags {
            if tag.scope_type == scope && in_scope(tag, initiative) {
                names.insert(tag.name.as_str(), tag);
            }
        }
    }
    names.into_values().collect()
}

pub fn reconcile_links(
    previous_tags: &[ProjectTag],
    tags: &[ProjectTag],
    initiatives: &[ProjectInitiativeRecord],
    todos: &[ProjectTodoRecord],
    links: &[ProjectTodoTag],
) -> Vec<ProjectTodoTag> {
    let mut result = BTreeSet::new();
    for todo in todos {
        let initiative = initiatives
            .iter()
            .find(|i| Some(&i.id) == todo.initiative_id.as_ref());
        let available = available_tags(tags, initiative);
        for link in links.iter().filter(|link| link.todo_id == todo.id) {
            let source = tags
                .iter()
                .find(|tag| tag.id == link.tag_id)
                .or_else(|| previous_tags.iter().find(|tag| tag.id == link.tag_id));
            if let Some(tag) =
                source.and_then(|source| available.iter().find(|tag| tag.name == source.name))
            {
                result.insert((todo.id.clone(), tag.id.clone()));
            }
        }
    }
    result
        .into_iter()
        .map(|(todo_id, tag_id)| ProjectTodoTag { todo_id, tag_id })
        .collect()
}

pub fn validate_selection(
    tags: &[ProjectTag],
    initiative: Option<&ProjectInitiativeRecord>,
    ids: &[String],
) -> anyhow::Result<()> {
    let allowed = available_tags(tags, initiative);
    for id in ids {
        let tag = tags
            .iter()
            .find(|tag| &tag.id == id)
            .ok_or(TagError::InvalidScope)?;
        anyhow::ensure!(
            in_scope(tag, initiative) && allowed.iter().any(|item| item.name == tag.name),
            TagError::InvalidScope
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_catalog_rejects_unknown_selections() {
        assert!(available_tags(&[], None).is_empty());
        assert!(validate_selection(&[], None, &["missing".into()]).is_err());
    }

    #[test]
    fn unassigned_todos_can_only_select_well_formed_global_tags() {
        let tags: Vec<ProjectTag> = serde_json::from_value(serde_json::json!([
            {"id":"global","scope_type":"global","scope_id":"","name":"通用"},
            {"id":"bad","scope_type":"global","scope_id":"owner","name":"错误"},
            {"id":"project","scope_type":"project","scope_id":"p","name":"项目"}
        ]))
        .unwrap();
        assert_eq!(available_tags(&tags, None), vec![&tags[0]]);
        assert!(validate_selection(&tags, None, &["global".into()]).is_ok());
        for id in ["bad", "project", "missing"] {
            assert!(validate_selection(&tags, None, &[id.into()]).is_err());
        }
    }
}
