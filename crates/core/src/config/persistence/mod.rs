//! Configuration persistence, separated from runtime configuration shape.
use super::*;

/// Selects where an explicit configuration save lands. `Auto` always preserves
/// legacy project/global routing. `EnvAware` opts an entry point into the active
/// Env and otherwise falls back to `Auto`. Project/Global never implicitly
/// intercept into an Env.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSaveRoute {
    Auto,
    EnvAware,
    ActiveEnv,
    Project,
    Global,
}

/// Resolved save destination suitable for TUI status and confirmation text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigSaveTarget {
    pub route: ConfigSaveRoute,
    pub path: PathBuf,
    pub active_env: Option<String>,
}

fn resolve_for_route(route: ConfigSaveRoute) -> Result<Option<local_env::ConfigEnvResolution>> {
    if matches!(
        route,
        ConfigSaveRoute::EnvAware | ConfigSaveRoute::ActiveEnv
    ) {
        local_env::resolve_active_config_env().map(Some)
    } else {
        Ok(None)
    }
}

fn active_target(active_env: Option<&local_env::ConfigEnvResolution>) -> Option<ConfigSaveTarget> {
    local_env::active_info(active_env).map(|active| ConfigSaveTarget {
        route: ConfigSaveRoute::ActiveEnv,
        path: active.path.join("config.json"),
        active_env: Some(active.name.clone()),
    })
}

fn require_active_target(
    active_env: Option<&local_env::ConfigEnvResolution>,
) -> Result<ConfigSaveTarget> {
    active_target(active_env).ok_or_else(|| CoreError::Config("no active configuration Env".into()))
}

fn editable_target(candidates: impl IntoIterator<Item = PathBuf>, fallback: PathBuf) -> PathBuf {
    for path in candidates {
        if path.exists() {
            if let Ok(raw) = std::fs::read_to_string(&path) {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) {
                    if merge::has_editable_key(&value) {
                        return path;
                    }
                }
            }
        }
    }
    fallback
}

fn auto_target(working_dir: &Path) -> ConfigSaveTarget {
    let path = editable_target(
        env::config_candidates(working_dir),
        working_dir.join("opencoder.json"),
    );
    let route = if path == working_dir.join(".opencoder/config.json")
        || path == working_dir.join("opencoder.json")
    {
        ConfigSaveRoute::Project
    } else {
        ConfigSaveRoute::Global
    };
    ConfigSaveTarget {
        route,
        path,
        active_env: None,
    }
}

fn project_target(working_dir: &Path) -> ConfigSaveTarget {
    let candidates = env::config_candidates(working_dir)
        .into_iter()
        .filter(|path| path.starts_with(working_dir))
        .take(2);
    ConfigSaveTarget {
        route: ConfigSaveRoute::Project,
        path: editable_target(candidates, working_dir.join("opencoder.json")),
        active_env: None,
    }
}

fn domain_target(
    working_dir: &Path,
    key: &str,
    route: ConfigSaveRoute,
    active_env: Option<&local_env::ConfigEnvResolution>,
) -> Result<PathBuf> {
    let active = || {
        if !domain::supports_active_env(key) {
            return None;
        }
        let active = local_env::active_info(active_env)?;
        domain::domain_file_name(key).map(|file| active.path.join(file))
    };
    let target = match route {
        ConfigSaveRoute::Auto => domain::write_target(working_dir, key),
        ConfigSaveRoute::EnvAware => active().or_else(|| domain::write_target(working_dir, key)),
        ConfigSaveRoute::ActiveEnv if domain::supports_active_env(key) => {
            require_active_target(active_env)?;
            active()
        }
        ConfigSaveRoute::ActiveEnv => None,
        ConfigSaveRoute::Project => {
            domain::is_domain_key(key).then(|| domain::project_domain_path(working_dir, key))
        }
        ConfigSaveRoute::Global => domain::global_domain_path(key),
    };
    target.ok_or_else(|| CoreError::Config(format!("no save target for domain `{key}`")))
}

fn target_for(
    working_dir: &Path,
    route: ConfigSaveRoute,
    active_env: Option<&local_env::ConfigEnvResolution>,
) -> Result<ConfigSaveTarget> {
    match route {
        ConfigSaveRoute::Auto => Ok(auto_target(working_dir)),
        ConfigSaveRoute::EnvAware => {
            Ok(active_target(active_env).unwrap_or_else(|| auto_target(working_dir)))
        }
        ConfigSaveRoute::ActiveEnv => require_active_target(active_env),
        ConfigSaveRoute::Project => Ok(project_target(working_dir)),
        ConfigSaveRoute::Global => Ok(ConfigSaveTarget {
            route,
            path: Config::global_config_path()?,
            active_env: None,
        }),
    }
}

impl Config {
    /// Legacy target query. Interactive Env routing is explicit via
    /// [`ConfigSaveRoute::EnvAware`] or [`ConfigSaveRoute::ActiveEnv`].
    pub fn save_target(working_dir: &Path) -> PathBuf {
        auto_target(working_dir).path
    }

    /// Resolve a config.json destination with route metadata. This performs no
    /// writes. `EnvAware` resolves to the actual ActiveEnv/Project/Global route.
    pub fn save_target_for(working_dir: &Path, route: ConfigSaveRoute) -> Result<ConfigSaveTarget> {
        let active_env = resolve_for_route(route)?;
        target_for(working_dir, route, active_env.as_ref())
    }

    /// Save through the legacy project/global target selection. Configuration
    /// entry points must opt into active-Env routing with `EnvAware`.
    pub fn save(working_dir: &Path, patch: &serde_json::Value) -> Result<PathBuf> {
        Self::save_with_route(working_dir, patch, ConfigSaveRoute::Auto)
    }

    /// Save with explicit routing. Domain keys follow the same route as the
    /// config remainder, and the returned path is the actual file written.
    pub fn save_with_route(
        working_dir: &Path,
        patch: &serde_json::Value,
        route: ConfigSaveRoute,
    ) -> Result<PathBuf> {
        // Resolve once before any write. Every domain and config target below
        // consumes this snapshot, so a mixed save cannot change Env mid-write.
        let active_env = resolve_for_route(route)?;
        Self::save_with_resolved_env(working_dir, patch, route, active_env.as_ref())
    }

    pub(super) fn save_with_resolved_env(
        working_dir: &Path,
        patch: &serde_json::Value,
        route: ConfigSaveRoute,
        active_env: Option<&local_env::ConfigEnvResolution>,
    ) -> Result<PathBuf> {
        let (remainder, domains) = domain::split_patch(patch);
        let remainder_target = if remainder
            .as_object()
            .is_some_and(|object| !object.is_empty())
        {
            Some(target_for(working_dir, route, active_env)?)
        } else {
            None
        };
        let mut last_domain = None;
        for (key, value) in &domains {
            let written = if route == ConfigSaveRoute::Auto {
                domain::save_domain(working_dir, key, value)
            } else {
                let target = domain_target(working_dir, key, route, active_env)?;
                domain::save_domain_to(target, key, value)
            }
            .map_err(|e| CoreError::Config(format!("save domain `{key}`: {e}")))?;
            last_domain = Some(written);
        }
        if let Some(target) = remainder_target {
            return Self::save_to(&target.path, &remainder);
        }
        if let Some(target) = last_domain {
            return Ok(target);
        }
        let target = target_for(working_dir, route, active_env)?;
        Self::save_to(&target.path, patch)
    }

    /// Merge a patch into canonical global config, ignoring project and Env.
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
        super::project::validate(&root)?;
        if let Some(servers) = root.get("mcp_servers").and_then(|v| v.as_object()) {
            if let Some((offending, existing)) = mcp_guard::mcp_name_collision(servers) {
                return Err(CoreError::Config(mcp_guard::conflict_message(
                    &offending, &existing,
                )));
            }
        }
        model_guard::validate_model_values(&root)?;
        let pretty = serde_json::to_string_pretty(&root)?;
        write_config_save(target, pretty.as_bytes())?;
        Ok(target.to_path_buf())
    }
}
