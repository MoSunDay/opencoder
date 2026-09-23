//! Configuration persistence, separated from runtime configuration shape.
use super::*;

impl Config {
    /// Pick the file to persist config edits to. Rule (project-first, global
    /// fallback): the first existing candidate that already holds any of the
    /// editable keys; if none, create the project-local `./opencoder.json`.
    pub fn save_target(working_dir: &Path) -> PathBuf {
        let candidates = env::config_candidates(working_dir);
        // candidates are ordered project-first (index 0) → global-last, which
        // is exactly the priority we want for picking a save target.
        for p in &candidates {
            if p.exists() {
                if let Ok(raw) = std::fs::read_to_string(p) {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                        if merge::has_editable_key(&v) {
                            return p.clone();
                        }
                    }
                }
            }
        }
        // Nothing editable on disk yet → create the project-local opencoder.json
        // at the working-dir root (more idiomatic than .opencoder/config.json).
        working_dir.join("opencoder.json")
    }

    /// Split-routing save (分流): top-level domain keys (`mcp_servers` /
    /// `cli` / `skills` / `autopilot`) are written to their dedicated domain
    /// files (`mcp.json` / `cli.json` / `skills.json` / `ap.json`); the
    /// remainder follows the
    /// normal [`save_target`](Self::save_target) + [`save_to`] config.json
    /// flow.
    ///
    /// Return-path semantics: a non-empty config remainder writes config.json
    /// and returns its path (domain writes still happen); a patch containing
    /// only domain keys returns the last domain write target and never
    /// creates a config.json; an empty patch with no domain keys keeps the
    /// legacy config.json-only behavior.
    pub fn save(working_dir: &Path, patch: &serde_json::Value) -> Result<PathBuf> {
        let (remainder, domains) = domain::split_patch(patch);
        let mut last_domain: Option<PathBuf> = None;
        for (key, value) in &domains {
            let target = domain::save_domain(working_dir, key, value)
                .map_err(|e| CoreError::Config(format!("save domain file for `{key}`: {e}")))?;
            last_domain = Some(target);
        }
        if remainder.as_object().is_some_and(|o| !o.is_empty()) {
            let target = Self::save_target(working_dir);
            return Self::save_to(&target, &remainder);
        }
        if let Some(target) = last_domain {
            return Ok(target);
        }
        let target = Self::save_target(working_dir);
        Self::save_to(&target, patch)
    }

    /// Merge a patch into the canonical global config, regardless of project
    /// config precedence. Used by first-run onboarding; normal `/model` saves
    /// continue to use [`save`](Self::save).
    pub fn save_global(patch: &serde_json::Value) -> Result<PathBuf> {
        let _ = Self::ensure_global_config()?;
        let target = Self::global_config_path()?;
        Self::save_to(&target, patch)
    }

    pub(super) fn save_to(target: &Path, patch: &serde_json::Value) -> Result<PathBuf> {
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut root: serde_json::Value = if target.exists() {
            let raw = std::fs::read_to_string(target)
                .map_err(|e| CoreError::Config(format!("read config {}: {e}", target.display())))?;
            match serde_json::from_str::<serde_json::Value>(&raw) {
                Ok(v) => v,
                Err(e) => {
                    // Don't silently destroy a corrupt file — surface the
                    // error. An empty/whitespace-only file is treated as an
                    // empty object (matches a freshly-created config).
                    if raw.trim().is_empty() {
                        serde_json::json!({})
                    } else {
                        return Err(CoreError::Config(format!(
                            "config file {} is corrupt: {e}; refusing to overwrite",
                            target.display()
                        )));
                    }
                }
            }
        } else {
            serde_json::json!({})
        };
        merge::merge_json(&mut root, patch);
        crate::provider::validate_protocol_patch(&root)?;
        // MCP name-collision guard (bug #14): two `mcp_servers` names that
        // normalize to the same tool prefix would shadow each other's tools
        // at registration. Defensive for paths that still carry the key
        // through config.json (`save_global` / the empty-patch fallback) —
        // normal saves route `mcp_servers` to mcp.json, guarded in
        // `domain::save_domain`. Runs before any write: nothing half-done.
        if let Some(servers) = root.get("mcp_servers").and_then(|v| v.as_object()) {
            if let Some((offending, existing)) = mcp_guard::mcp_name_collision(servers) {
                return Err(CoreError::Config(mcp_guard::conflict_message(
                    &offending, &existing,
                )));
            }
        }
        // Guard: refuse to persist a malformed `model` (e.g. `m/g`). Such a value
        // would make every downstream request fail silently (`model_id()` resolves
        // to a single char). Surface the error so the caller shows it to the user
        // instead of corrupting the config file. See is_suspicious_model for the
        // predicate.
        if let Some(model) = root.get("model").and_then(|v| v.as_str()) {
            if is_suspicious_model(model) {
                return Err(CoreError::Config(format!(
                    "refusing to write malformed `model` value `{model}`: expected \
                     `provider/model` with each side at least 2 chars (e.g. \
                     `openai/gpt-4o`); edit the `model` field in your config file"
                )));
            }
        }
        let pretty = serde_json::to_string_pretty(&root)?;
        // E-1: a save landing in the active env dir must honor the 0o600
        // owner-only contract (api keys live in these files).
        write_config_save(target, &pretty)?;
        Ok(target.to_path_buf())
    }
}
