//! Local named configuration Envs (not process environment variables).
//!
//! An Env lives at `~/.opencoder/envs/<name>/`. The owner-only
//! `~/.opencoder/active_env` marker selects the interactive Env. All lookups
//! are fresh, so switching A -> B -> A never reuses a cached configuration.

use std::path::{Path, PathBuf};

use crate::error::{CoreError, Result};

mod preflight;
mod resolution;

pub(crate) use resolution::{
    active_config_env_resolution, active_info, resolve_active_config_env, ConfigEnvResolution,
};

const ACTIVE_MARKER: &str = "active_env";
const ENV_FILES: [&str; 3] = ["config.json", "cli.json", "skills.json"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigEnvInfo {
    pub name: String,
    pub path: PathBuf,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigEnvStatus {
    pub active: Option<String>,
    pub active_path: Option<PathBuf>,
    pub stale_marker: bool,
    pub marker_path: PathBuf,
}

fn envs_root() -> Result<PathBuf> {
    super::env::global_opencoder_home()
        .map(|home| home.join("envs"))
        .ok_or_else(|| CoreError::Config("cannot resolve home for local Envs".into()))
}

fn marker_path() -> Result<PathBuf> {
    super::env::global_opencoder_home()
        .map(|home| home.join(ACTIVE_MARKER))
        .ok_or_else(|| CoreError::Config("cannot resolve home for active Env".into()))
}

pub fn validate_config_env_name(name: &str) -> Result<()> {
    let valid = !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_alphanumeric())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        && crate::platform::fs::valid_component(name);
    if valid {
        Ok(())
    } else {
        Err(CoreError::Config(format!(
            "invalid Env name `{name}`: use 1-64 ASCII letters, digits, '-' or '_', starting with a letter or digit"
        )))
    }
}

pub fn config_env_dir(name: &str) -> Result<PathBuf> {
    validate_config_env_name(name)?;
    Ok(envs_root()?.join(name))
}

fn real_directory(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|metadata| metadata.is_dir() && !crate::platform::fs::is_link(&metadata))
        .unwrap_or(false)
}

fn read_marker() -> Result<Option<Vec<u8>>> {
    match std::fs::read(marker_path()?) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn active_config_env() -> Option<ConfigEnvInfo> {
    match active_config_env_resolution() {
        ConfigEnvResolution::Active(active) => Some(active),
        ConfigEnvResolution::NoEnv | ConfigEnvResolution::Stale => None,
    }
}

pub fn config_env_status() -> Result<ConfigEnvStatus> {
    let (active, active_path, stale_marker) = match resolve_active_config_env()? {
        ConfigEnvResolution::Active(active) => (Some(active.name), Some(active.path), false),
        ConfigEnvResolution::NoEnv => (None, None, false),
        ConfigEnvResolution::Stale => (None, None, true),
    };
    Ok(ConfigEnvStatus {
        active,
        active_path,
        stale_marker,
        marker_path: marker_path()?,
    })
}
fn info(name: &str, active: bool) -> Result<ConfigEnvInfo> {
    Ok(ConfigEnvInfo {
        name: name.to_owned(),
        path: config_env_dir(name)?,
        active,
    })
}

pub fn list_config_envs() -> Result<Vec<ConfigEnvInfo>> {
    let root = envs_root()?;
    let active = active_config_env().map(|env| env.name);
    let entries = match std::fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if validate_config_env_name(&name).is_ok() {
                names.push(name);
            }
        }
    }
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let is_active = active.as_deref() == Some(name.as_str());
            info(&name, is_active)
        })
        .collect()
}

pub fn create_config_env(name: &str) -> Result<ConfigEnvInfo> {
    let directory = config_env_dir(name)?;
    let root = envs_root()?;
    crate::platform::fs::ensure_private_directory(&root)?;
    crate::platform::fs::create_private_directory(&directory).map_err(|error| {
        CoreError::Config(format!(
            "create Env `{name}` at {}: {error}",
            directory.display()
        ))
    })?;
    let result = ENV_FILES.iter().try_for_each(|file| {
        super::write_config_save(&directory.join(file), b"{}\n")?;
        Ok::<(), std::io::Error>(())
    });
    if let Err(error) = result {
        let _ = std::fs::remove_dir_all(&directory);
        return Err(error.into());
    }
    let active = active_config_env().is_some_and(|env| env.name == name);
    info(name, active)
}

fn restore_marker(path: &Path, original: Option<&[u8]>) -> std::io::Result<()> {
    match original {
        Some(bytes) => super::write_config_save(path, bytes),
        None => match std::fs::remove_file(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            result => result,
        },
    }
}

pub fn activate_config_env(name: &str) -> Result<ConfigEnvStatus> {
    let directory = config_env_dir(name)?;
    if !real_directory(&directory) {
        return Err(CoreError::Config(format!("Env `{name}` does not exist")));
    }
    let marker = marker_path()?;
    let original = read_marker()?;
    preflight::validate_env(&directory)?;
    let attempted = (|| {
        super::write_config_save(&marker, format!("{name}\n").as_bytes())?;
        config_env_status()
    })();
    match attempted {
        Ok(status) => Ok(status),
        Err(error) => match restore_marker(&marker, original.as_deref()) {
            Ok(()) => Err(error),
            Err(rollback) => Err(CoreError::Config(format!(
                "Env activation failed ({error}); marker rollback also failed: {rollback}"
            ))),
        },
    }
}

pub fn deactivate_config_env() -> Result<ConfigEnvStatus> {
    let marker = marker_path()?;
    match std::fs::remove_file(marker) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    config_env_status()
}

pub fn delete_config_env(name: &str) -> Result<()> {
    let directory = config_env_dir(name)?;
    if !real_directory(&directory) {
        return Err(CoreError::Config(format!("Env `{name}` does not exist")));
    }
    if active_config_env().is_some_and(|env| env.name == name) {
        deactivate_config_env()?;
    }
    std::fs::remove_dir_all(directory)?;
    Ok(())
}
