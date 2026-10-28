//! Privilege-drop descriptor for harness subprocesses.
//!
//! Plain data: the Linux identity an execution's tool subprocesses should
//! run as. It is inert on non-Linux hosts (nothing consumes it there) but
//! serializes everywhere so persisted harness state stays portable across
//! platforms.
use serde::{Deserialize, Serialize};

/// Linux user/group identity for a harness run. `groups` are supplementary
/// group ids (the primary `gid` may also appear in it). `username` is
/// informational for diagnostics; uid/gid are authoritative.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunAsIdentity {
    pub uid: u32,
    pub gid: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_round_trips_and_keeps_minimal_form_stable() {
        let identity = RunAsIdentity {
            uid: 900,
            gid: 900,
            groups: vec![900, 910],
            username: Some("ocop-operator-fixture".into()),
        };
        let value = serde_json::to_value(&identity).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "uid": 900, "gid": 900, "groups": [900, 910],
                "username": "ocop-operator-fixture"
            })
        );
        assert_eq!(
            serde_json::from_value::<RunAsIdentity>(value).unwrap(),
            identity
        );
        // Minimal persisted form: groups/username optional.
        let minimal = serde_json::from_value::<RunAsIdentity>(serde_json::json!({
            "uid": 1, "gid": 1
        }))
        .unwrap();
        assert_eq!(minimal.groups, Vec::<u32>::new());
        assert_eq!(minimal.username, None);
    }
}
