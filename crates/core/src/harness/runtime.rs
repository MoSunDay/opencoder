//! Private, versioned execution configuration; Agent cards contain references only.
use super::CodexSettings;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Versioned<T> {
    pub revision: u64,
    pub settings: T,
}

#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RuntimeSettings {
    pub profiles: BTreeMap<String, Versioned<CodexSettings>>,
    pub runners: BTreeMap<String, Versioned<RunnerSettings>>,
}

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RunnerSettings {
    pub parent_unit: Option<String>,
    pub command: Vec<String>,
    pub workdir: PathBuf,
    pub envs: BTreeMap<String, String>,
    /// Immutable installation files checked before admission and every execution.
    pub files: BTreeMap<PathBuf, String>,
}

impl std::fmt::Debug for RunnerSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunnerSettings")
            .field("files", &self.files.len())
            .field("environment_keys", &self.envs.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl RunnerSettings {
    pub fn validate(&self) -> Result<(), String> {
        if self.parent_unit.as_deref().is_some_and(|unit| {
            !unit.ends_with(".service")
                || unit.len() > 128
                || !unit
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.@".contains(&b))
        }) {
            return Err("invalid Runner parent service".into());
        }
        if self.command.is_empty()
            || !std::path::Path::new(&self.command[0]).is_absolute()
            || self.command.iter().any(|a| a.contains('\0'))
            || !self.workdir.is_absolute()
        {
            return Err("Runner requires an absolute executable and working directory".into());
        }
        if self.files.is_empty()
            || self.files.iter().any(|(p, hash)| {
                !p.is_absolute() || hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit())
            })
        {
            return Err("Runner requires installation files with SHA-256 checksums".into());
        }
        for (key, value) in &self.envs {
            super::validate_env(key, value)?;
        }
        if serde_json::to_vec(self).map_err(|e| e.to_string())?.len() > 256 * 1024 {
            return Err("Runner configuration exceeds 256 KiB".into());
        }
        Ok(())
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
