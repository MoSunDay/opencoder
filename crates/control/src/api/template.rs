//! Resolve a template and its environment before sending the immutable snapshot.
use anyhow::Result;
use opencoder_core::share_fs::*;
#[cfg(test)]
use serde_json::json;
use serde_json::Value;
use std::path::Path;

pub(super) fn snapshot(root: &Path, name: &str, version: &str) -> Result<Value> {
    let spec =
        opencoder_todos::directory::load_bound(&todo_version_dir(root, name, version)?, root)?;
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
        write(
            env_context_path(root, "test").unwrap(),
            json!({"tools":[], "env_vars":{"Z_LAST":"1","A_FIRST":"2"}}),
        );
        let pinned = snapshot(root, "demo", "v1").unwrap();
        assert_eq!(pinned["metadata"]["env"], "test");
        assert_eq!(pinned["metadata"]["env_tools"], json!([]));
        assert_eq!(
            pinned["metadata"]["env_vars"],
            json!({"A_FIRST":"2","Z_LAST":"1"})
        );
        // Invalid env_vars (bad key / non-string value) rejects dispatch.
        write(
            env_context_path(root, "test").unwrap(),
            json!({"tools":[], "env_vars":{"bad-key":"1"}}),
        );
        assert!(snapshot(root, "demo", "v1")
            .unwrap_err()
            .to_string()
            .contains("env_vars"));
        write(
            env_context_path(root, "test").unwrap(),
            json!({"tools":[], "env_vars":{"GOOD":42}}),
        );
        assert!(snapshot(root, "demo", "v1")
            .unwrap_err()
            .to_string()
            .contains("env_vars"));
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
