//! Independent human ownership and subscriptions, shared by all project records.
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectPeople {
    #[serde(default)]
    pub owner_user_name: Option<String>,
    #[serde(default)]
    pub follower_user_names: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectPeoplePatch {
    #[serde(
        default,
        deserialize_with = "present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub owner_user_name: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub follower_user_names: Option<Vec<String>>,
}

fn present_option<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(d).map(Some)
}

#[derive(Debug, thiserror::Error)]
#[error("project user does not exist: {0}")]
pub struct InvalidProjectUser(pub String);

impl ProjectPeople {
    /// Deterministic set representation; a human may hold both roles.
    pub fn normalized(mut self) -> Self {
        self.follower_user_names = self
            .follower_user_names
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        self
    }

    pub fn replacement(&self) -> ProjectPeoplePatch {
        ProjectPeoplePatch {
            owner_user_name: Some(self.owner_user_name.clone()),
            follower_user_names: Some(self.follower_user_names.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn people_sets_keep_owner_and_patch_distinguishes_omission_from_clear() {
        let value: ProjectPeople = serde_json::from_value(
            json!({"owner_user_name":"alice","follower_user_names":["bob","alice","bob"]}),
        )
        .unwrap();
        let value = value.normalized();
        assert_eq!(value.follower_user_names, ["alice", "bob"]);
        assert_eq!(
            value.replacement().owner_user_name,
            Some(Some("alice".into()))
        );
        let absent: ProjectPeoplePatch = serde_json::from_value(json!({})).unwrap();
        assert!(absent.owner_user_name.is_none());
        let clear: ProjectPeoplePatch =
            serde_json::from_value(json!({"owner_user_name":null,"follower_user_names":[]}))
                .unwrap();
        assert_eq!(clear.owner_user_name, Some(None));
        assert_eq!(clear.follower_user_names, Some(vec![]));
        assert_eq!(serde_json::to_value(absent).unwrap(), json!({}));
    }
}
