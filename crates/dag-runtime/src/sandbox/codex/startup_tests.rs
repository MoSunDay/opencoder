use super::*;
use serde_json::json;
use std::os::unix::fs::PermissionsExt;

struct Fixture {
    temp: tempfile::TempDir,
    rootfs: PathBuf,
    command: Vec<String>,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let launcher = root.join("launcher");
        let rootfs = root.join("rootfs");
        for path in [
            launcher.clone(),
            rootfs.join("usr/share/opencoder"),
            root.join("home"),
            root.join("clients/opencoder"),
            root.join("connections"),
        ] {
            std::fs::create_dir_all(path).unwrap();
        }
        let program = launcher.join("run");
        std::fs::write(&program, "#!/usr/bin/python3\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let guest = guest_path(&rootfs, program.to_str().unwrap()).unwrap();
        std::fs::create_dir_all(guest.parent().unwrap()).unwrap();
        std::fs::copy(&program, guest).unwrap();
        let codex = "/usr/bin/codex";
        std::fs::create_dir_all(rootfs.join("usr/bin")).unwrap();
        std::fs::write(rootfs.join("usr/bin/codex"), "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(
            rootfs.join("usr/bin/codex"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        std::fs::write(
            rootfs.join("usr/share/opencoder/codex-startup.json"),
            json!({
                "schema_version":1,"program":program,"codex":codex,
                "files":{"run":format!("{:x}",Sha256::digest(std::fs::read(&program).unwrap()))}
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(root.join("proxy"), "http://fixture.invalid").unwrap();
        std::fs::write(root.join("deployment.json"), "{}").unwrap();
        std::fs::write(root.join("token"), "private-fixture-token").unwrap();
        std::fs::write(
            root.join("clients/opencoder/ctl.json"),
            json!({"server":"http://fixture.invalid", "token_file":root.join("token")}).to_string(),
        )
        .unwrap();
        std::fs::write(
            root.join("connections/token"),
            "private-connection-fixture-token",
        )
        .unwrap();
        std::fs::write(
            root.join("connections/service.json"),
            json!({"endpoint":"http://service.invalid", "token_file":"token"}).to_string(),
        )
        .unwrap();
        let config = root.join("config.json");
        std::fs::write(
            &config,
            json!({"deployment_config":root.join("deployment.json"),
            "proxy_file":root.join("proxy"),"codex_home":root.join("home"),
            "sqlite_home":root.join("home/sqlite"),"client_config_home":root.join("clients"),
            "connection_files":[root.join("connections/service.json")],"codex":codex})
            .to_string(),
        )
        .unwrap();
        Self {
            temp,
            rootfs,
            command: vec![
                program.display().to_string(),
                "--config".into(),
                config.display().to_string(),
            ],
        }
    }
}

#[test]
fn managed_startup_freezes_config_and_mounts_only_declared_private_files() {
    let fixture = Fixture::new();
    let startup = resolve(&fixture.rootfs, &fixture.command).unwrap().unwrap();
    assert_eq!(startup.mounts.len(), 6);
    for relative in ["connections/service.json", "connections/token"] {
        assert!(startup.mounts.iter().any(|mount| {
            mount.source == fixture.temp.path().join(relative) && !mount.directory
        }));
    }
    let root = fixture.temp.path().join("run");
    let mut mounts = vec![];
    for step in ["first", "second"] {
        std::fs::create_dir_all(root.join("private/codex").join(step)).unwrap();
        startup
            .prepare(&root, step, &fixture.rootfs, &mut mounts)
            .unwrap();
    }
    assert_eq!(mounts.len(), 7);
    assert!(mounts
        .iter()
        .all(|m| m["options"].as_array().unwrap().contains(&json!("ro"))));
    assert!(!mounts
        .iter()
        .any(|m| m["destination"].as_str().unwrap().ends_with("/clients")));
    let before = std::fs::read(root.join("private/codex/first/startup.json")).unwrap();
    std::fs::write(&fixture.command[2], "changed after acceptance").unwrap();
    assert_eq!(
        std::fs::read(root.join("private/codex/first/startup.json")).unwrap(),
        before
    );
    assert!(!String::from_utf8(before)
        .unwrap()
        .contains("private-fixture-token"));
}

#[test]
fn managed_startup_rejects_drift_aliases_and_repeated_state_root() {
    let fixture = Fixture::new();
    let path = Path::new(&fixture.command[2]);
    let original: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let mut repeated = original.clone();
    repeated["state_dir"] = json!("/second-root");
    std::fs::write(path, repeated.to_string()).unwrap();
    assert!(resolve(&fixture.rootfs, &fixture.command).is_err());
    std::fs::write(path, original.to_string()).unwrap();
    std::fs::write(&fixture.command[0], "changed host launcher").unwrap();
    assert!(resolve(&fixture.rootfs, &fixture.command).is_err());
}

#[test]
fn managed_startup_only_reads_explicit_connection_files() {
    let fixture = Fixture::new();
    let path = Path::new(&fixture.command[2]);
    let mut config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    std::fs::write(
        fixture.temp.path().join("connections/service.json"),
        "invalid",
    )
    .unwrap();
    assert!(resolve(&fixture.rootfs, &fixture.command).is_err());
    for connections in [Some(json!([])), None] {
        if let Some(files) = connections {
            config["connection_files"] = files;
        } else {
            config.as_object_mut().unwrap().remove("connection_files");
        }
        std::fs::write(path, config.to_string()).unwrap();
        let startup = resolve(&fixture.rootfs, &fixture.command).unwrap().unwrap();
        assert_eq!(startup.mounts.len(), 4);
        assert!(!startup.mounts.iter().any(|mount| {
            mount
                .source
                .starts_with(fixture.temp.path().join("connections"))
        }));
    }
}

#[test]
fn managed_startup_rejects_invalid_connection_declarations_and_missing_tokens() {
    let fixture = Fixture::new();
    let path = Path::new(&fixture.command[2]);
    let mut config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let connection = fixture.temp.path().join("connections/service.json");
    let alias = fixture.temp.path().join("connections/alias.json");
    std::os::unix::fs::symlink(&connection, &alias).unwrap();
    for files in [
        json!(null),
        json!("not-an-array"),
        json!([42]),
        json!(["relative.json"]),
        json!([fixture.temp.path().join("missing.json")]),
        json!([fixture.temp.path().join("connections")]),
        json!([alias]),
    ] {
        config["connection_files"] = files;
        std::fs::write(path, config.to_string()).unwrap();
        assert!(resolve(&fixture.rootfs, &fixture.command).is_err());
    }
    config["connection_files"] = json!([connection]);
    std::fs::write(path, config.to_string()).unwrap();
    let token = fixture.temp.path().join("connections/token");
    let before = std::fs::read(&token).unwrap();
    resolve(&fixture.rootfs, &fixture.command).unwrap();
    assert_eq!(std::fs::read(&token).unwrap(), before);
    std::fs::remove_file(&token).unwrap();
    assert!(resolve(&fixture.rootfs, &fixture.command).is_err());
    std::os::unix::fs::symlink(fixture.temp.path().join("token"), &token).unwrap();
    assert!(resolve(&fixture.rootfs, &fixture.command).is_err());
}
