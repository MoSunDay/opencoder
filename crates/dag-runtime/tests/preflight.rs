use opencoder_dag_runtime::sandbox::run::preflight;
use serde_json::Value;
use std::{
    os::unix::fs::{symlink, PermissionsExt},
    path::Path,
};

fn runner(root: &Path, name: &str, info: &Value) {
    std::fs::create_dir_all(root).unwrap();
    let path = root.join(name);
    std::fs::write(&path, format!("#!/bin/sh\nprintf '%s\\n' '{}'\n", info)).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn runners_dir_rejects_missing_mixed_version_and_linked_runners_before_admission() {
    let directory = tempfile::tempdir().unwrap();
    let mut config = opencoder_core::Config::default();
    assert_eq!(
        preflight(&config).unwrap_err().to_string(),
        "DAG runners_dir is required"
    );
    config.dag.runners_dir = Some(directory.path().to_path_buf());
    let info = serde_json::to_value(opencoder_core::version::build_info()).unwrap();
    assert!(preflight(&config)
        .unwrap_err()
        .to_string()
        .contains("dag-runner"));
    runner(directory.path(), "dag-runner", &info);
    assert!(preflight(&config)
        .unwrap_err()
        .to_string()
        .contains("agent-step-runner"));
    runner(directory.path(), "agent-step-runner", &info);
    assert_eq!(
        preflight(&config).unwrap_err().to_string(),
        "DAG workspace_dir is required"
    );
    for field in [
        "git_commit",
        "protocol_version",
        "release_compatibility",
        "brain_schema_version",
        "spa_sha256",
    ] {
        let mut changed = info.clone();
        changed[field] = Value::Null;
        runner(directory.path(), "agent-step-runner", &changed);
        assert!(preflight(&config)
            .unwrap_err()
            .to_string()
            .contains("version does not match"));
    }
    std::fs::remove_file(directory.path().join("agent-step-runner")).unwrap();
    symlink("dag-runner", directory.path().join("agent-step-runner")).unwrap();
    assert!(preflight(&config)
        .unwrap_err()
        .to_string()
        .contains("regular file"));
}
