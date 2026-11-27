//! One-shot active-Env resolution shared by a complete config operation.

use crate::error::Result;

use super::ConfigEnvInfo;

/// The marker state observed by one config load/save operation.
///
/// `Stale` is a readable marker that does not identify a real Env directory;
/// callers retain their normal non-Env fallback. Only I/O failures are errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ConfigEnvResolution {
    Active(ConfigEnvInfo),
    NoEnv,
    Stale,
}

/// Borrow the active Env without consulting the filesystem again.
pub(crate) fn active_info(resolution: Option<&ConfigEnvResolution>) -> Option<&ConfigEnvInfo> {
    match resolution {
        Some(ConfigEnvResolution::Active(active)) => Some(active),
        Some(ConfigEnvResolution::NoEnv | ConfigEnvResolution::Stale) | None => None,
    }
}

/// Resolve the marker exactly once, preserving all non-NotFound I/O failures.
pub(crate) fn resolve_active_config_env() -> Result<ConfigEnvResolution> {
    let Some(bytes) = super::read_marker()? else {
        return Ok(ConfigEnvResolution::NoEnv);
    };
    let Some(name) = std::str::from_utf8(&bytes).ok().map(str::trim) else {
        return Ok(ConfigEnvResolution::Stale);
    };
    if super::validate_config_env_name(name).is_err() {
        return Ok(ConfigEnvResolution::Stale);
    }
    let path = super::config_env_dir(name)?;
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_dir() && !crate::platform::fs::is_link(&metadata) => {
            Ok(ConfigEnvResolution::Active(ConfigEnvInfo {
                name: name.to_owned(),
                path,
                active: true,
            }))
        }
        Ok(_) => Ok(ConfigEnvResolution::Stale),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(ConfigEnvResolution::Stale)
        }
        Err(error) => Err(error.into()),
    }
}

/// Compatibility resolution for legacy helpers whose signatures cannot return
/// marker errors. Strict Core operations never use this after preflight.
pub(crate) fn active_config_env_resolution() -> ConfigEnvResolution {
    match resolve_active_config_env() {
        Ok(ConfigEnvResolution::Stale) => {
            tracing::warn!("ignoring stale active Env marker");
            ConfigEnvResolution::Stale
        }
        Ok(resolution) => resolution,
        Err(error) => {
            tracing::warn!("ignoring unreadable active Env marker: {error}");
            ConfigEnvResolution::NoEnv
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serde_json::{json, Value};

    use super::{resolve_active_config_env, ConfigEnvResolution};
    use crate::config::env::scoped_config_home;
    use crate::config::local_env::{activate_config_env, create_config_env};
    use crate::config::{Config, ConfigSaveRoute};

    fn write_json(path: &Path, value: Value) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, value.to_string()).unwrap();
    }

    fn read_json(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn resolution_distinguishes_absent_stale_active_and_io_errors() {
        let home = tempfile::tempdir().unwrap();
        let _guard = scoped_config_home(home.path().to_path_buf());
        assert_eq!(
            resolve_active_config_env().unwrap(),
            ConfigEnvResolution::NoEnv
        );

        let marker = super::super::marker_path().unwrap();
        std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
        std::fs::write(&marker, "missing\n").unwrap();
        assert_eq!(
            resolve_active_config_env().unwrap(),
            ConfigEnvResolution::Stale
        );

        let env = create_config_env("active").unwrap();
        activate_config_env("active").unwrap();
        match resolve_active_config_env().unwrap() {
            ConfigEnvResolution::Active(active) => {
                assert_eq!(active.name, "active");
                assert_eq!(active.path, env.path);
            }
            other => panic!("expected active Env, got {other:?}"),
        }

        std::fs::remove_file(&marker).unwrap();
        std::fs::create_dir(&marker).unwrap();
        assert!(resolve_active_config_env().is_err());
    }

    #[test]
    fn resolved_env_survives_marker_failure_during_load_and_mixed_save() {
        let home = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let _guard = scoped_config_home(home.path().to_path_buf());
        let env = create_config_env("stable").unwrap();
        write_json(
            &env.path.join("config.json"),
            json!({"model": "initial/model"}),
        );
        write_json(
            &env.path.join("cli.json"),
            json!({"initial-cli": {"enabled": true, "content": "initial"}}),
        );
        write_json(
            &env.path.join("skills.json"),
            json!({"initial-skill": {"enabled": true}}),
        );
        activate_config_env("stable").unwrap();
        let resolved = resolve_active_config_env().unwrap();

        // Simulate a failure immediately after the operation's one resolution.
        // Both internal pipelines must keep using `resolved`, not read again.
        let marker = super::super::marker_path().unwrap();
        std::fs::remove_file(&marker).unwrap();
        std::fs::create_dir(&marker).unwrap();

        let loaded =
            Config::load_with_resolved_env(work.path(), None, true, Some(&resolved)).unwrap();
        assert_eq!(loaded.model, "initial/model");
        assert_eq!(loaded.enabled_cli()[0].0, "initial-cli");
        assert_eq!(loaded.enabled_skill_names(), ["initial-skill"]);

        Config::save_with_resolved_env(
            work.path(),
            &json!({
                "model": "saved/model",
                "cli": {"saved-cli": {"enabled": true, "content": "saved"}},
                "skills": {"saved-skill": {"enabled": true}}
            }),
            ConfigSaveRoute::EnvAware,
            Some(&resolved),
        )
        .unwrap();

        assert_eq!(
            read_json(&env.path.join("config.json"))["model"],
            "saved/model"
        );
        assert!(read_json(&env.path.join("cli.json"))["saved-cli"].is_object());
        assert!(read_json(&env.path.join("skills.json"))["saved-skill"].is_object());
        for wrong_scope in [
            work.path().join("opencoder.json"),
            work.path().join(".opencoder/cli.json"),
            work.path().join(".opencoder/skills.json"),
            home.path().join(".opencoder/config.json"),
            home.path().join(".opencoder/cli.json"),
            home.path().join(".opencoder/skills.json"),
        ] {
            assert!(!wrong_scope.exists(), "wrote {}", wrong_scope.display());
        }
    }
}
