//! Typed Codex settings. Kept outside the NFS resource tree and pinned per run.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CodexSettings {
    pub auth_slot: Option<u32>,
    pub executable: Option<String>,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub sandbox_mode: Option<String>,
    pub approval_policy: Option<String>,
    pub envs: BTreeMap<String, String>,
}

impl std::fmt::Debug for CodexSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodexSettings")
            .field("model", &self.model)
            .field("environment_keys", &self.envs.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl CodexSettings {
    pub fn validate(&self) -> Result<(), String> {
        if self.auth_slot == Some(0) {
            return Err("Codex auth_slot must be positive".into());
        }
        for (name, value) in [("executable", &self.executable), ("model", &self.model)] {
            if value
                .as_ref()
                .is_some_and(|s| s.trim().is_empty() || s.contains('\0'))
            {
                return Err(format!("invalid Codex {name}"));
            }
        }
        for (name, value, choices) in [
            (
                "reasoning_effort",
                &self.reasoning_effort,
                &["none", "minimal", "low", "medium", "high", "xhigh"][..],
            ),
            (
                "sandbox_mode",
                &self.sandbox_mode,
                &["read-only", "workspace-write", "danger-full-access"][..],
            ),
            (
                "approval_policy",
                &self.approval_policy,
                &["never", "on-request", "untrusted"][..],
            ),
        ] {
            if value.as_deref().is_some_and(|v| !choices.contains(&v)) {
                return Err(format!("invalid Codex {name}"));
            }
        }
        for (key, value) in &self.envs {
            super::validate_env(key, value)?;
        }
        if serde_json::to_vec(self).map_err(|e| e.to_string())?.len() > 64 * 1024 {
            return Err("Codex settings exceed 64 KiB".into());
        }
        Ok(())
    }

    /// Values are separate argv entries, never interpreted by a shell.
    pub fn config_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(slot) = self.auth_slot {
            args.extend(["--auth-slot".into(), slot.to_string()]);
        }
        for (key, value) in [
            ("model_reasoning_effort", &self.reasoning_effort),
            ("sandbox_mode", &self.sandbox_mode),
            ("approval_policy", &self.approval_policy),
        ] {
            if let Some(value) = value {
                args.extend([
                    "-c".into(),
                    format!("{key}={}", serde_json::to_string(value).unwrap()),
                ]);
            }
        }
        args
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_validate_and_preserve_literal_environment() {
        let mut settings = CodexSettings {
            reasoning_effort: Some("high".into()),
            envs: BTreeMap::from([("NOTE".into(), "literal $(command)=value".into())]),
            ..Default::default()
        };
        settings.validate().unwrap();
        assert_eq!(
            settings.config_args(),
            ["-c", "model_reasoning_effort=\"high\""]
        );
        assert!(!format!("{settings:?}").contains("$(command)"));
        settings.sandbox_mode = Some("typo".into());
        assert!(settings.validate().is_err());
        assert!(serde_json::from_str::<CodexSettings>(r#"{"args":["--help"]}"#).is_err());
    }
}
