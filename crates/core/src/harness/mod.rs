//! Execution selection and private, durable harness state.
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf, str::FromStr};
pub mod scope;
mod settings;
pub use settings::CodexSettings;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Harness {
    #[default]
    Opencoder,
    Codex,
}

impl Harness {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Opencoder => "opencoder",
            Self::Codex => "codex",
        }
    }
}
impl FromStr for Harness {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "opencoder" => Ok(Self::Opencoder),
            "codex" => Ok(Self::Codex),
            _ => Err(format!(
                "unknown harness '{s}'; expected opencoder or codex"
            )),
        }
    }
}

/// Private state: never serialize this object into public session responses.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HarnessRuntime {
    pub codex: Option<CodexSettings>,
    pub harness: Harness,
    pub thread_id: Option<String>,
    pub fork_from: Option<String>,
    pub resource_root: Option<PathBuf>,
    pub envs: BTreeMap<String, String>,
    pub model: Option<String>,
    pub last_input_id: Option<String>,
    pub in_flight: bool,
}

pub fn agent_harness(name: &str) -> Harness {
    crate::agent::read_agent_meta(name)
        .map(|m| m.harness)
        .unwrap_or_default()
}

/// Only fresh Codex sessions take managed defaults. Existing threads keep their snapshot.
pub fn pin_settings(runtime: &mut HarnessRuntime, settings: Option<&CodexSettings>) {
    if runtime.harness != Harness::Codex
        || runtime.codex.is_some()
        || runtime.thread_id.is_some()
        || runtime.last_input_id.is_some()
    {
        return;
    }
    if let Some(settings) = settings {
        let mut envs = settings.envs.clone();
        envs.extend(runtime.envs.clone());
        runtime.envs = envs;
        runtime.model = settings.model.clone();
        runtime.codex = Some(settings.clone());
    }
}

pub fn parse_env(s: &str) -> Result<(String, String), String> {
    let (key, value) = s.split_once('=').ok_or("environment must be KEY=VALUE")?;
    validate_env(key, value)?;
    Ok((key.into(), value.into()))
}

pub fn validate_env(key: &str, value: &str) -> Result<(), String> {
    if key.is_empty() || key.contains(['=', '\0']) || value.contains('\0') {
        return Err("invalid environment variable name or value".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn environment_preserves_payload_and_rejects_invalid_pairs() {
        assert_eq!(parse_env("A= x=y ").unwrap(), ("A".into(), " x=y ".into()));
        assert_eq!(parse_env("A=").unwrap().1, "");
        for s in ["A", "=x", "A=x\0"] {
            assert!(parse_env(s).is_err());
        }
    }
    #[test]
    fn old_runtime_defaults_to_native_and_unknown_harness_fails() {
        assert_eq!(
            serde_json::from_str::<HarnessRuntime>("{}")
                .unwrap()
                .harness,
            Harness::Opencoder
        );
        assert!(serde_json::from_str::<HarnessRuntime>(r#"{"harness":"typo"}"#).is_err());
    }
}
