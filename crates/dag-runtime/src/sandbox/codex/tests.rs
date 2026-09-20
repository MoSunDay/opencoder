#![cfg(unix)]

use super::*;
use opencoder_core::harness::CodexSettings;

fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).into(), (*v).into()))
        .collect()
}

#[test]
fn credential_precedence_matches_host_process_inheritance() {
    let inherited = map(&[("HOME", "/node/user")]);
    assert_eq!(
        credential_home(&map(&[]), &inherited).unwrap(),
        Path::new("/node/user/.codex")
    );
    assert_eq!(
        credential_home(&map(&[("HOME", "/profile")]), &inherited).unwrap(),
        Path::new("/profile/.codex")
    );
    let inherited = map(&[("HOME", "/node/user"), ("CODEX_HOME", "/node/login")]);
    assert_eq!(
        credential_home(&map(&[("HOME", "/profile")]), &inherited).unwrap(),
        Path::new("/node/login")
    );
    assert_eq!(
        credential_home(&map(&[("CODEX_HOME", "/explicit")]), &inherited).unwrap(),
        Path::new("/explicit")
    );
    for value in ["", "relative/login"] {
        assert!(credential_home(&map(&[("CODEX_HOME", value)]), &inherited).is_err());
    }
    assert!(credential_home(&map(&[]), &map(&[])).is_err());
}

#[test]
fn guest_settings_keep_profile_and_credentials_private() {
    let runtime = HarnessRuntime {
        harness: Harness::Codex,
        model: Some("profile-model".into()),
        envs: map(&[
            ("HTTPS_PROXY", "profile-proxy"),
            ("NOTE", "literal $(value)"),
        ]),
        codex: Some(CodexSettings {
            auth_slot: Some(2),
            reasoning_effort: Some("high".into()),
            sandbox_mode: Some("read-only".into()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let guest = guest_runtime(
        runtime,
        &map(&[
            ("HTTPS_PROXY", "host-proxy"),
            ("OPENAI_API_KEY", "fixture-key"),
        ]),
    )
    .unwrap();
    assert_eq!(guest.envs["CODEX_HOME"], HOME_MOUNT);
    assert_eq!(guest.envs["HOME"], GUEST_HOME);
    assert_eq!(guest.envs["HTTPS_PROXY"], "profile-proxy");
    assert_eq!(guest.envs["OPENAI_API_KEY"], "fixture-key");
    assert_eq!(guest.model.as_deref(), Some("profile-model"));
    let settings = guest.codex.unwrap();
    assert_eq!(settings.auth_slot, Some(2));
    assert_eq!(settings.sandbox_mode.as_deref(), Some("read-only"));
    assert_eq!(settings.approval_policy.as_deref(), Some("never"));
    assert_eq!(settings.executable.as_deref(), Some(DEFAULT_BINARY));
}

#[test]
fn profile_resolution_validates_guest_binary_and_builds_private_mounts() {
    use opencoder_core::harness::Versioned;
    use serde_json::json;
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let home = root.join("login");
    let agents = root.join("agents");
    let rootfs = root.join("rootfs");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("auth.json"), "fixture-login").unwrap();
    std::fs::create_dir_all(agents.join("check")).unwrap();
    std::fs::write(
        agents.join("check/meta.json"),
        json!({"name":"check","harness":"codex","harness_profile":"selected"}).to_string(),
    )
    .unwrap();
    let mut config = opencoder_core::Config::default();
    config.agent.agents_dir = Some(agents);
    assert!(resolve(&config, "check", &rootfs)
        .unwrap_err_text()
        .contains("profile selected unavailable"));
    config.agent.runtime.profiles.insert(
        "selected".into(),
        Versioned {
            revision: 1,
            settings: CodexSettings {
                model: Some("selected-model".into()),
                envs: BTreeMap::from([
                    ("CODEX_HOME".into(), home.display().to_string()),
                    ("PRIVATE_VALUE".into(), "fixture-private-value".into()),
                ]),
                ..Default::default()
            },
        },
    );
    assert!(resolve(&config, "check", &rootfs)
        .unwrap_err_text()
        .contains("executable missing in rootfs"));
    std::fs::create_dir_all(rootfs.join("usr/bin")).unwrap();
    std::fs::write(rootfs.join("usr/bin/codex"), "#!/bin/sh\nexit 0").unwrap();
    std::fs::set_permissions(
        rootfs.join("usr/bin/codex"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let launch = resolve(&config, "check", &rootfs).unwrap().unwrap();
    assert_eq!(launch.home, home);
    assert_eq!(launch.runtime.model.as_deref(), Some("selected-model"));
    let run = root.join("run");
    std::fs::create_dir_all(&run).unwrap();
    let spec = crate::sandbox::oci::BundleSpec {
        run_root: run.clone(),
        step_slug: "check".into(),
        command: vec!["/usr/bin/agent-step-runner".into()],
        env: vec![],
        timeout_hint: None,
        knowledge: None,
        agents: None,
        argv: crate::sandbox::oci::ArgvStyle::Direct,
    };
    let bundle = launch.write_bundle(&root.join("bundle"), &spec).unwrap();
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(bundle.join("config.json")).unwrap()).unwrap();
    assert!(!value.to_string().contains("fixture-private-value"));
    let mounts = value["mounts"].as_array().unwrap();
    let login = mounts
        .iter()
        .find(|m| m["destination"] == HOME_MOUNT)
        .unwrap();
    assert_eq!(login["source"], json!(home));
    assert!(login["options"].as_array().unwrap().contains(&json!("rw")));
    let manifest = mounts
        .iter()
        .find(|m| m["destination"] == LAUNCH_MOUNT)
        .unwrap();
    assert!(manifest["options"]
        .as_array()
        .unwrap()
        .contains(&json!("ro")));
    assert_eq!(
        std::fs::metadata(bundle.join("codex-private/launch.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::read_dir(run).unwrap().count(),
        0,
        "private launch must not enter DAG artifacts"
    );
    assert_eq!(
        std::fs::read_to_string(home.join("auth.json")).unwrap(),
        "fixture-login"
    );
    assert!(resolve(&config, "act", &rootfs).unwrap().is_none());
    std::fs::remove_file(rootfs.join("usr/bin/codex")).unwrap();
    std::os::unix::fs::symlink("/bin/sh", rootfs.join("usr/bin/codex")).unwrap();
    assert!(resolve(&config, "check", &rootfs)
        .unwrap_err_text()
        .contains("symlinks"));
}

// Keep private launch settings out of panic Debug output.
trait ErrorText {
    fn unwrap_err_text(self) -> String;
}
impl<T> ErrorText for Result<T> {
    fn unwrap_err_text(self) -> String {
        match self {
            Err(e) => format!("{e:#}"),
            Ok(_) => panic!("expected error"),
        }
    }
}
