//! Resolve a template and its environment before sending the immutable snapshot.
use anyhow::{bail, Context, Result};
use opencoder_core::share_fs::*;
use serde_json::{json, Value};
use std::path::Path;

pub(super) fn snapshot(root: &Path, name: &str, version: &str) -> Result<Value> {
    let context = read_json_opt(&todo_context_path(root, name, version)?)?
        .context("template version not found")?;
    let mut spec: opencoder_todos::WorkflowSpec = serde_json::from_value(context)?;
    let binding = read_json_opt(&todo_env_binding_path(root, name, version)?)?;
    if let Some(env) = binding
        .as_ref()
        .and_then(|v| v["env"].as_str())
        .filter(|s| !s.is_empty())
    {
        let context =
            read_json_opt(&env_context_path(root, env)?)?.context("bound environment missing")?;
        let tools = context.get("tools").cloned().unwrap_or(json!([]));
        for tool in tools
            .as_array()
            .context("environment tools must be an array")?
        {
            let reference = tool
                .as_str()
                .context("environment tool reference must be a string")?;
            if resolve_tool_ref(root, reference).is_err() {
                bail!("environment tool missing: {reference}");
            }
        }
        if !spec.metadata.is_object() {
            spec.metadata = json!({});
        }
        spec.metadata["env"] = json!(env);
        spec.metadata["env_tools"] = tools;
    }
    opencoder_todos::domain::validate_spec(&spec)?;
    Ok(serde_json::to_value(spec)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bound_environment_is_pinned_and_missing_tools_reject_dispatch() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let write = |path: std::path::PathBuf, value: Value| {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
        };
        write(
            todo_context_path(root, "demo", "v1").unwrap(),
            json!({"schema_version":1,"id":"demo","name":"demo","objective":"check","todos":[{"id":"t1","title":"check","requirement_background":"test","depends_on":[],"instructions":"check","agent":"act","max_attempts":1,"acceptance":{"criteria":"checked"}}]}),
        );
        write(
            todo_env_binding_path(root, "demo", "v1").unwrap(),
            json!({"env":"test"}),
        );
        write(env_context_path(root, "test").unwrap(), json!({"tools":[]}));
        let pinned = snapshot(root, "demo", "v1").unwrap();
        assert_eq!(pinned["metadata"]["env"], "test");
        assert_eq!(pinned["metadata"]["env_tools"], json!([]));
        write(
            env_context_path(root, "test").unwrap(),
            json!({"tools":["missing/tool"]}),
        );
        assert!(snapshot(root, "demo", "v1")
            .unwrap_err()
            .to_string()
            .contains("tool missing"));
        assert_eq!(pinned["metadata"]["env_tools"], json!([]));
    }
}
