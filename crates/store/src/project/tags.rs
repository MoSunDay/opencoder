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

pub fn available_tags<'a>(
    tags: &'a [ProjectTag],
    initiative: Option<&ProjectInitiativeRecord>,
) -> Vec<&'a ProjectTag> {
    let Some(initiative) = initiative else {
        return vec![];
    };
    let mut names = BTreeMap::new();
    for scope in ["project", "initiative"] {
        for tag in tags {
            if tag.scope_type == scope
                && if scope == "project" {
                    initiative.goal_id.as_deref() == Some(tag.scope_id.as_str())
                } else {
                    tag.scope_id == initiative.id
                }
            {
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
        let in_scope = initiative.is_some_and(|i| {
            (tag.scope_type == "initiative" && tag.scope_id == i.id)
                || (tag.scope_type == "project"
                    && i.goal_id.as_deref() == Some(tag.scope_id.as_str()))
        });
        anyhow::ensure!(
            in_scope && allowed.iter().any(|item| item.name == tag.name),
            TagError::InvalidScope
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_initiative_has_no_available_tags() {
        assert!(available_tags(&[], None).is_empty());
        assert!(validate_selection(&[], None, &["missing".into()]).is_err());
    }
}
