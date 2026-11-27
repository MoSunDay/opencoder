//! Strict validation performed before changing the interactive active Env.

use std::path::{Path, PathBuf};

use crate::error::{CoreError, Result};

use super::super::{agent, domain, model_guard, project, Config};

fn read_object(path: &Path) -> Result<serde_json::Value> {
    let raw = std::fs::read_to_string(path)
        .map_err(|error| CoreError::Config(format!("read Env file {}: {error}", path.display())))?;
    let value: serde_json::Value = serde_json::from_str(&raw).map_err(|error| {
        CoreError::Config(format!("invalid Env JSON {}: {error}", path.display()))
    })?;
    if !value.is_object() {
        return Err(CoreError::Config(format!(
            "Env file {} must contain a JSON object",
            path.display()
        )));
    }
    Ok(value)
}

fn validate_config(path: &Path) -> Result<()> {
    let mut value = read_object(path)?;
    crate::provider::validate_protocol_patch(&value)?;
    agent::validate_private_settings(&value)?;
    project::validate(&value)?;
    model_guard::validate_model_values(&value)?;
    // Normal config merging treats a null modes block as an explicit reset.
    // Normalize only that value for serde's stricter standalone Config decode;
    // every other malformed value still fails preflight.
    if value.get("modes").is_some_and(serde_json::Value::is_null) {
        value["modes"] = serde_json::json!({});
    }
    serde_json::from_value::<Config>(value).map_err(|error| {
        CoreError::Config(format!("invalid Env config {}: {error}", path.display()))
    })?;
    Ok(())
}

fn validate_domain(path: &Path, key: &str) -> Result<()> {
    let value = read_object(path)?;
    domain::validate_value(key, &value)
        .map_err(|error| CoreError::Config(format!("{}: {error}", path.display())))
}

/// Validate config and Env-aware domain files without changing the marker or
/// any Env content. Missing optional domain files are valid empty domains.
pub(super) fn validate_env(directory: &Path) -> Result<()> {
    let config = directory.join("config.json");
    if config.exists() {
        validate_config(&config)?;
    }
    let files: Vec<(PathBuf, &str)> = domain::ENV_DOMAIN_FILES
        .iter()
        .map(|(key, file)| (directory.join(file), *key))
        .collect();
    for (path, key) in files {
        if path.exists() {
            validate_domain(&path, key)?;
        }
    }
    Ok(())
}
