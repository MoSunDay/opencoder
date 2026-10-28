//! `operator` config block: node-plane knobs for operator executions.
//!
//! Pure serde data; provisioning behavior lives in the worker.

use serde::{Deserialize, Serialize};

/// Per-execution Linux identity provisioning for operator executions.
///
/// - `Enforced` (default): every operator execution provisions a dedicated
///   Linux system user; a node without the capability (non-root, missing
///   `useradd`) rejects operator executions.
/// - `Off`: keep node-user subprocesses (local development escape valve).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LinuxIdentityMode {
    #[default]
    Enforced,
    Off,
}

/// Access tier of an operator execution's private directories.
///
/// - `ReadWrite` (default): the execution owns its home/workspace and
///   writes through a private `TMPDIR`.
/// - `ReadOnly`: hardened inspection runs — node-owned, non-writable
///   directories and no `TMPDIR`, so tool writes fail.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OperatorAccessMode {
    #[default]
    ReadWrite,
    ReadOnly,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperatorConfig {
    #[serde(default)]
    pub linux_identity: LinuxIdentityMode,
    #[serde(default)]
    pub access: OperatorAccessMode,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_identity_defaults_to_enforced_and_parses_both_modes() {
        assert_eq!(
            serde_json::from_str::<OperatorConfig>("{}")
                .unwrap()
                .linux_identity,
            LinuxIdentityMode::Enforced
        );
        assert_eq!(
            serde_json::from_str::<OperatorConfig>(r#"{"linux_identity":"off"}"#)
                .unwrap()
                .linux_identity,
            LinuxIdentityMode::Off
        );
        assert_eq!(
            serde_json::to_value(OperatorConfig::default()).unwrap(),
            serde_json::json!({"linux_identity":"enforced","access":"readwrite"})
        );
        assert!(serde_json::from_str::<OperatorConfig>(r#"{"linux_identity":"nope"}"#).is_err());
    }

    #[test]
    fn access_defaults_to_read_write_and_parses_both_tiers() {
        assert_eq!(
            serde_json::from_str::<OperatorConfig>("{}").unwrap().access,
            OperatorAccessMode::ReadWrite
        );
        assert_eq!(
            serde_json::from_str::<OperatorConfig>(r#"{"access":"readonly"}"#)
                .unwrap()
                .access,
            OperatorAccessMode::ReadOnly
        );
        assert!(serde_json::from_str::<OperatorConfig>(r#"{"access":"nope"}"#).is_err());
    }
}
