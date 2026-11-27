//! Consistent live-config snapshots for callers that also need raw domains.

use std::path::Path;

use serde_json::Value;

use super::{agent, domain, env, local_env, merge, project, Config};
use crate::error::Result;

impl Config {
    /// Load the live configuration and the requested effective raw domain
    /// values from one strict active-Env resolution. Returned values follow
    /// `domain_keys` order; an unknown or unresolved domain yields `None`.
    /// MCP and schedules retain their legacy project/global routing.
    pub fn load_with_domain_values(
        working_dir: &Path,
        domain_keys: &[&str],
    ) -> Result<(Config, Vec<Option<Value>>)> {
        let active_env = local_env::resolve_active_config_env()?;
        Self::load_snapshot_with_resolved_env(
            working_dir,
            None,
            true,
            Some(&active_env),
            domain_keys,
        )
    }

    pub(super) fn load_with_resolved_env(
        working_dir: &Path,
        home: Option<&Path>,
        apply_environment: bool,
        active_env: Option<&local_env::ConfigEnvResolution>,
    ) -> Result<Config> {
        Self::load_snapshot_with_resolved_env(working_dir, home, apply_environment, active_env, &[])
            .map(|(config, _)| config)
    }

    fn load_snapshot_with_resolved_env(
        working_dir: &Path,
        home: Option<&Path>,
        apply_environment: bool,
        active_env: Option<&local_env::ConfigEnvResolution>,
        domain_keys: &[&str],
    ) -> Result<(Config, Vec<Option<Value>>)> {
        let mut cfg = Config::default();
        // Merge ALL existing candidates, least-specific first so project files
        // override the global base (matches opencoder). This lets ~/.opencoder
        // provide the provider+key while a project opencoder.json overrides only
        // the model — `opencoder` then runs directly from any directory.
        let mut candidates = env::candidates_for_load(working_dir, home, active_env);
        candidates.reverse(); // global first, project last (wins)
        for p in candidates {
            if p.exists() {
                let raw = std::fs::read_to_string(&p)?;
                let parsed: serde_json::Value = serde_json::from_str(&raw)?;
                crate::provider::validate_protocol_patch(&parsed)?;
                agent::validate_private_settings(&parsed)?;
                project::validate(&parsed)?;
                if !parsed.is_object() {
                    // A valid-JSON-but-not-object file (e.g. `[1,2]` or
                    // `"foo"`) falls through `merge_into` silently. Warn so the
                    // misconfiguration is visible instead of dropped.
                    let kind = match &parsed {
                        serde_json::Value::Null => "null",
                        serde_json::Value::Bool(_) => "bool",
                        serde_json::Value::Number(_) => "number",
                        serde_json::Value::String(_) => "string",
                        serde_json::Value::Array(_) => "array",
                        serde_json::Value::Object(_) => "object",
                    };
                    tracing::warn!(
                        "config file {} is valid JSON but not an object (got \
                         {}); ignoring",
                        p.display(),
                        kind
                    );
                }
                merge::merge_into(&mut cfg, parsed);
            }
        }
        // Domain files use one effective file per key. Cli/skills may use an
        // active Env and XDG fallback; MCP/schedules keep project/global only.
        // No cross-file key merge. Capture raw outputs from these same reads so
        // callers never perform a second domain resolution.
        let mut raw_domains = vec![None; domain_keys.len()];
        for (key, _) in domain::DOMAIN_FILES {
            let value = domain::read_effective_for_load(working_dir, key, home, active_env);
            for (output, requested) in raw_domains.iter_mut().zip(domain_keys) {
                if *requested == key {
                    *output = value.clone();
                }
            }
            if let Some(value) = value {
                domain::apply_domain(&mut cfg, key, &value);
            }
        }
        agent::validate_private_settings(&serde_json::json!({"agent": &cfg.agent}))?;
        let config = Self::finalize(cfg, apply_environment)?;
        Ok((config, raw_domains))
    }
}
