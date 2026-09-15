//! Private, versioned execution configuration; Agent cards contain references only.
use super::CodexSettings;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Versioned<T> {
    pub revision: u64,
    pub settings: T,
}

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeSettings {
    pub profiles: BTreeMap<String, Versioned<CodexSettings>>,
    /// Opaque historical fields round-trip through persisted execution snapshots.
    /// Only profiles are interpreted by current execution code.
    #[serde(flatten)]
    pub archived: BTreeMap<String, serde_json::Value>,
}

impl std::fmt::Debug for RuntimeSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeSettings")
            .field("profiles", &self.profiles)
            .field("archived_field_count", &self.archived.len())
            .finish()
    }
}

/// Resolve only the explicitly selected profile. Missing references are errors.
pub fn agent_settings<'a>(
    config: &'a crate::Config,
    agent: &str,
) -> Result<Option<&'a CodexSettings>, String> {
    match crate::agent::read_agent_meta(agent).and_then(|meta| meta.harness_profile) {
        Some(name) => config
            .agent
            .runtime
            .profiles
            .get(&name)
            .map(|profile| Some(&profile.settings))
            .ok_or_else(|| format!("Codex profile {name} unavailable for agent {agent}")),
        None => Ok(config.agent.codex.as_ref()),
    }
}

pub fn pin_agent_settings(
    runtime: &mut super::HarnessRuntime,
    config: &crate::Config,
    agent: &str,
) -> Result<(), String> {
    if runtime.harness == super::Harness::Codex
        && runtime.codex.is_none()
        && runtime.thread_id.is_none()
        && runtime.last_input_id.is_none()
    {
        crate::agent::scope::with_root_sync(config.agent.agents_dir.clone(), || {
            super::pin_settings(runtime, agent_settings(config, agent)?);
            Ok::<_, String>(())
        })?;
    }
    Ok(())
}
