use super::*;
use opencoder_core::harness::{CodexSettings, Versioned};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::PathBuf;

/// A fixture host root with the mapped directories of a merged-usr node and a
/// runners directory; `render` reads the node's real mountinfo, under which no
/// fixture path carries a nested mount.
fn host(root: &Path) -> PathBuf {
    let host = root.join("host");
    for directory in [
        "etc",
        "lib64",
        "sbin",
        "usr/bin",
        "usr/lib",
        "usr/lib64",
        "usr/sbin",
    ] {
        std::fs::create_dir_all(host.join(directory)).unwrap();
    }
    std::os::unix::fs::symlink("usr/bin", host.join("bin")).unwrap();
    std::os::unix::fs::symlink("usr/lib", host.join("lib")).unwrap();
    std::fs::create_dir_all(root.join("runners")).unwrap();
    let binary = host.join("usr/bin/codex");
    std::fs::write(&binary, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
    host
}

fn bind_destinations(spec: &serde_json::Value) -> Vec<String> {
    spec["mounts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|mount| mount["type"] == "bind")
        .map(|mount| mount["destination"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn codex_steps_share_one_home_at_the_original_node_path() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let home = root.join("login");
    for directory in ["login", "agents/check", "workspace", "private"] {
        std::fs::create_dir_all(root.join(directory)).unwrap();
    }
    let host = host(root);
    std::fs::write(
        root.join("agents/check/meta.json"),
        json!({"name":"check","harness":"codex","harness_profile":"selected"}).to_string(),
    )
    .unwrap();
    let mut config = opencoder_core::Config::default();
    config.agent.agents_dir = Some(root.join("agents"));
    config.dag.runners_dir = Some(root.join("runners"));
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
    let spec = render(root, &host, &config, &run, root.metadata().unwrap().uid()).unwrap();
    // The root is the empty anchor sealed read-only behind a tmpfs `/`
    // carrying the host mapping and the runners.
    assert_eq!(spec["root"], json!({"path":"root","readonly":true}));
    assert_eq!(spec["mounts"][0]["destination"], "/");
    assert_eq!(spec["mounts"][0]["type"], "tmpfs");
    let mounts = spec["mounts"].as_array().unwrap();
    let bind = |guest: &str| {
        mounts
            .iter()
            .find(|mount| mount["destination"] == guest)
            .unwrap_or_else(|| panic!("no mount at {guest}"))
    };
    assert_eq!(
        bind("/bin")["source"],
        json!(host.join("usr/bin").canonicalize().unwrap())
    );
    // Real directories bind themselves; only links are bound through their target.
    assert_eq!(bind("/etc")["source"], json!(host.join("etc")));
    assert_eq!(bind("/lib64")["source"], json!(host.join("lib64")));
    assert_eq!(
        bind("/run/opencoder/bin")["source"],
        json!(root.join("runners"))
    );
    for guest in ["/bin", "/etc", "/lib64", "/usr", "/run/opencoder/bin"] {
        let options = bind(guest)["options"].as_array().unwrap();
        assert!(
            options.contains(&json!("ro")) && !options.contains(&json!("rbind")),
            "{guest}"
        );
    }
    assert_eq!(
        spec["process"]["args"],
        json!(["/run/opencoder/bin/dag-runner", "init"])
    );
    assert_eq!(
        spec["process"]["env"],
        json!([format!("PATH={}", host::GUEST_PATH)])
    );
    let homes: Vec<_> = mounts
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
fn run_uses_shallow_binds_when_knowledge_has_no_submounts() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    for name in ["workspace", "private", "knowledge", "agents", "credentials"] {
        std::fs::create_dir_all(root.join(name)).unwrap();
    }
    let host = host(root);
    let mut config = opencoder_core::Config::default();
    config.dag.knowledge_root = Some(root.join("knowledge"));
    config.agent.agents_dir = Some(root.join("agents"));
    config.dag.runners_dir = Some(root.join("runners"));
    config.dag.execution_private_root = Some(root.join("credentials"));
    let run: DagClaimedRun = serde_json::from_value(json!({
        "run_id":"bind-check", "dag_id":"bind-check", "created_at":0,
        "spec":{"name":"bind-check", "steps":[
            {"name":"binary","kind":{"type":"binary","resource":"fixture@v1","args":[]}}
        ]}
    }))
    .unwrap();
    let spec = render(root, &host, &config, &run, root.metadata().unwrap().uid()).unwrap();
    let devices: Vec<_> = spec["mounts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|mount| mount["destination"].as_str().unwrap().starts_with("/dev"))
        .collect();
    assert_eq!(devices[0]["destination"], "/dev");
    assert_eq!(devices[1]["destination"], "/dev/pts");
    assert_eq!(devices[1]["type"], "devpts");
    let options = devices[1]["options"].as_array().unwrap();
    for required in [
        "newinstance",
        "ptmxmode=0666",
        "mode=0620",
        "nosuid",
        "noexec",
    ] {
        assert!(options.contains(&json!(required)));
    }
    assert!(!spec["process"]["capabilities"]["effective"]
        .as_array()
        .unwrap()
        .contains(&json!("CAP_SYS_ADMIN")));
    let destinations = bind_destinations(&spec);
    let unique: std::collections::BTreeSet<_> = destinations.iter().collect();
    assert_eq!(
        unique.len(),
        destinations.len(),
        "mount destinations are unique"
    );
    for guest in [
        "/workspace",
        "/run/opencoder-private",
        crate::exec::native::io::KNOWLEDGE_MOUNT,
        crate::exec::native::io::AGENTS_MOUNT,
        opencoder_core::fleet::private_files::GUEST_ROOT,
        "/workspace/binary/meta",
    ] {
        assert!(
            destinations.iter().any(|destination| destination == guest),
            "{guest}"
        );
    }
    let mounts = spec["mounts"].as_array().unwrap();
    for mount in mounts.iter().filter(|mount| mount["type"] == "bind") {
        let options = mount["options"].as_array().unwrap();
        assert!(options.contains(&json!("bind")));
        assert!(!options.contains(&json!("rbind")));
        let writable = mount["destination"] == "/workspace";
        assert!(options.contains(&json!(if writable { "rw" } else { "ro" })));
    }
}
