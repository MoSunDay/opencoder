use super::*;
use opencoder_core::harness::{CodexSettings, Versioned};
use std::os::unix::fs::{MetadataExt, PermissionsExt};

#[test]
fn codex_steps_share_one_home_at_the_original_node_path() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let home = root.join("login");
    for directory in [
        "login",
        "agents/check",
        "bundle/rootfs/usr/bin",
        "workspace",
        "private",
    ] {
        std::fs::create_dir_all(root.join(directory)).unwrap();
    }
    let binary = root.join("bundle/rootfs/usr/bin/codex");
    std::fs::write(&binary, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(
        root.join("agents/check/meta.json"),
        json!({"name":"check","harness":"codex","harness_profile":"selected"}).to_string(),
    )
    .unwrap();
    let mut config = opencoder_core::Config::default();
    config.agent.agents_dir = Some(root.join("agents"));
    config.agent.runtime.profiles.insert(
        "selected".into(),
        Versioned {
            revision: 1,
            settings: CodexSettings {
                envs: [("CODEX_HOME".into(), home.display().to_string())].into(),
                ..Default::default()
            },
        },
    );
    let run: DagClaimedRun = serde_json::from_value(json!({
        "run_id":"run-check", "dag_id":"dag-check", "created_at":0,
        "spec":{"name":"check", "steps":[
            {"name":"first","kind":{"type":"agent","agent":"check","prompt":"one"}},
            {"name":"second","kind":{"type":"agent","agent":"check","prompt":"two"}}
        ]}
    }))
    .unwrap();
    let spec = render(root, &config, &run, root.metadata().unwrap().uid()).unwrap();
    let homes: Vec<_> = spec["mounts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|mount| mount["source"] == json!(home))
        .collect();
    assert_eq!(homes.len(), 1);
    assert_eq!(homes[0]["destination"], json!(home));
    for step in ["first", "second"] {
        let launch: serde_json::Value = serde_json::from_slice(
            &std::fs::read(root.join("private/codex").join(step).join("launch.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(launch["envs"]["CODEX_HOME"], json!(home));
        assert_eq!(launch["codex"]["envs"]["CODEX_HOME"], json!(home));
    }
    assert!(!root.join("workspace/first/launch.json").exists());
}

#[test]
fn run_binds_only_declared_directories_without_inheriting_host_submounts() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    for name in [
        "bundle/rootfs",
        "workspace",
        "private",
        "knowledge",
        "agents",
        "credentials",
    ] {
        std::fs::create_dir_all(root.join(name)).unwrap();
    }
    let mut config = opencoder_core::Config::default();
    config.dag.knowledge_root = Some(root.join("knowledge"));
    config.agent.agents_dir = Some(root.join("agents"));
    config.dag.execution_private_root = Some(root.join("credentials"));
    let run: DagClaimedRun = serde_json::from_value(json!({
        "run_id":"bind-check", "dag_id":"bind-check", "created_at":0,
        "spec":{"name":"bind-check", "steps":[
            {"name":"binary","kind":{"type":"binary","resource":"fixture@v1","args":[]}}
        ]}
    }))
    .unwrap();
    let spec = render(root, &config, &run, root.metadata().unwrap().uid()).unwrap();
    let binds: Vec<_> = spec["mounts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|mount| mount["type"] == "bind")
        .collect();
    assert_eq!(binds.len(), 6);
    for mount in binds {
        let options = mount["options"].as_array().unwrap();
        assert!(options.contains(&json!("bind")));
        assert!(!options.contains(&json!("rbind")));
        let writable = mount["destination"] == "/workspace";
        assert!(options.contains(&json!(if writable { "rw" } else { "ro" })));
    }
}
