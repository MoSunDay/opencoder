//! Resolve a directory binding once, before the workflow is frozen.
use crate::WorkflowSpec;
use anyhow::{Context, Result};
use opencoder_core::share_fs::*;
use serde_json::{json, Value};
use std::path::Path;

pub fn apply_environment(spec: &mut WorkflowSpec, root: &Path, env: &str) -> Result<()> {
    let context = read_json_opt(&env_context_path(root, env)?)?
        .with_context(|| format!("env.json: 环境不存在: {env}"))?;
    let tools = context.get("tools").cloned().unwrap_or(json!([]));
    for tool in tools
        .as_array()
        .context("env.json: environment tools must be an array")?
    {
        let reference = tool
            .as_str()
            .context("env.json: tool reference must be a string")?;
        resolve_tool_ref(root, reference)
            .with_context(|| format!("env.json: environment tool missing: {reference}"))?;
    }
    let vars = crate::domain::env_vars_from_context(&context)?;
    anyhow::ensure!(
        spec.metadata.is_object() || spec.metadata.is_null(),
        "workflow.json: metadata must be an object when binding an environment"
    );
    if spec.metadata.is_null() {
        spec.metadata = json!({});
    }
    spec.metadata["env"] = json!(env);
    spec.metadata["env_tools"] = tools;
    spec.metadata["env_vars"] = crate::domain::env_vars_metadata(vars);
    crate::domain::validate_spec(spec)
}

pub fn load_bound(path: &Path, share: &Path) -> Result<WorkflowSpec> {
    let mut spec = super::load(path)?;
    if path.is_dir() {
        let files = super::read_files(path)?;
        let binding: Value = serde_json::from_str(&files["env.json"])?;
        if let Some(env) = binding["env"].as_str() {
            apply_environment(&mut spec, share, env)?;
        }
    }
    Ok(spec)
}
