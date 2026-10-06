use opencoder_core::{scoped_config_home, Config};
use serde_json::{json, Value};
use std::path::Path;

fn write(path: &Path, value: &Value) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, value.to_string()).unwrap();
}

fn base() -> Value {
    json!({"agent": {
        "codex": {
            "executable": "C:\\Tools\\codex.exe", "auth_slot": 2,
            "model": "global", "approval_policy": "never",
            "envs": {"HTTPS_PROXY": "http://localhost:8080", "KEEP": "global"}
        },
        "runtime": {"profiles": {
            "global": {"revision": 1, "settings": {"model": "global"}},
            "shared": {"revision": 1, "settings": {
                "executable": "old.exe", "envs": {"OLD": "old"}
            }}
        }}
    }})
}

#[test]
fn project_codex_overlay_preserves_global_launch_settings_and_frozen_operator() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let work = temp.path().join("work");
    let _isolation = scoped_config_home(home.clone());
    write(&home.join(".opencoder/config.json"), &base());
    write(
        &work.join("opencoder.json"),
        &json!({"agent":{"codex":{"model":"project","envs":{"KEEP":"project"}}}}),
    );
    let loaded = Config::load_with_home(&work, Some(&home)).unwrap();
    let settings = loaded.agent.codex.as_ref().unwrap();
    assert_eq!(settings.model.as_deref(), Some("project"));
    assert_eq!(settings.executable.as_deref(), Some(r"C:\Tools\codex.exe"));
    assert_eq!(settings.auth_slot, Some(2));
    assert_eq!(settings.approval_policy.as_deref(), Some("never"));
    assert_eq!(settings.envs["HTTPS_PROXY"], "http://localhost:8080");
    assert_eq!(settings.envs["KEEP"], "project");
    assert_eq!(&settings.config_args()[..2], ["--auth-slot", "2"]);
    let operator = temp.path().join("operator");
    write(
        &operator.join("config.json"),
        &serde_json::to_value(&loaded).unwrap(),
    );
    let frozen = Config::load_operator(&operator).unwrap();
    assert_eq!(frozen.agent.codex, loaded.agent.codex);
    assert_eq!(frozen.agent.runtime, loaded.agent.runtime);
}

#[test]
fn project_profiles_preserve_other_names_and_replace_whole_revisions() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let work = temp.path().join("work");
    let _isolation = scoped_config_home(home.clone());
    write(&home.join(".opencoder/config.json"), &base());
    let patch = json!({"agent":{"runtime":{"profiles":{
        "shared":{"revision":2,"settings":{"model":"new"}},
        "project":{"revision":1,"settings":{"model":"project"}}
    }}}});
    write(&work.join("opencoder.json"), &patch);
    let loaded = Config::load_with_home(&work, Some(&home)).unwrap();
    let profiles = &loaded.agent.runtime.profiles;
    assert_eq!(profiles.len(), 3);
    assert_eq!(profiles["global"].settings.model.as_deref(), Some("global"));
    assert_eq!(profiles["shared"].revision, 2);
    assert_eq!(profiles["shared"].settings.model.as_deref(), Some("new"));
    assert!(profiles["shared"].settings.executable.is_none());
    assert!(profiles["shared"].settings.envs.is_empty());
    let original = Config::default().merged_with(&base());
    assert_eq!(
        original.merged_with(&patch).agent.runtime,
        loaded.agent.runtime
    );
    assert_eq!(original.agent.runtime.profiles["shared"].revision, 1);
}

#[test]
fn codex_patch_changes_only_explicit_fields_and_null_clears_settings() {
    let original = Config::default().merged_with(&base());
    let partial = original.merged_with(&json!({"agent":{"codex":{
        "model":"project","executable":null
    }}}));
    let settings = partial.agent.codex.as_ref().unwrap();
    assert_eq!(settings.model.as_deref(), Some("project"));
    assert!(settings.executable.is_none());
    assert_eq!(settings.auth_slot, Some(2));
    assert_eq!(settings.envs, original.agent.codex.as_ref().unwrap().envs);
    assert!(original.agent.codex.as_ref().unwrap().executable.is_some());
    assert!(partial
        .merged_with(&json!({"agent":{"codex":null}}))
        .agent
        .codex
        .is_none());
}

#[test]
fn partial_nfs_overlays_preserve_enabled_host_and_read_only_global_settings() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let work = temp.path().join("work");
    let _isolation = scoped_config_home(home.clone());
    let nfs = json!({"enabled":true,"host":"127.0.0.2","port":22000,"read_only":true});
    write(
        &home.join(".opencoder/config.json"),
        &json!({
            "agent":{"nfs":nfs}, "dag":{"nfs":nfs,"workspace_nfs":{
                "enabled":true,"host":"127.0.0.2","port":22000
            }}
        }),
    );
    write(
        &work.join("opencoder.json"),
        &json!({
            "agent":{"nfs":{"port":22001}},
            "dag":{"nfs":{"port":22002},"workspace_nfs":{"port":22003}}
        }),
    );
    let loaded = Config::load_with_home(&work, Some(&home)).unwrap();
    for value in [
        serde_json::to_value(&loaded.agent.nfs).unwrap(),
        serde_json::to_value(&loaded.dag.nfs).unwrap(),
    ] {
        assert_eq!(value["enabled"], true);
        assert_eq!(value["host"], "127.0.0.2");
        assert_eq!(value["read_only"], true);
    }
    assert_eq!(loaded.agent.nfs.port, 22001);
    assert_eq!(loaded.dag.nfs.port, 22002);
    assert_eq!(loaded.dag.workspace_nfs.port, 22003);
    assert!(loaded.dag.workspace_nfs.enabled);
    assert_eq!(loaded.dag.workspace_nfs.host, "127.0.0.2");
}

#[test]
fn resolved_private_overlays_reject_combined_dispatch_budgets_without_exposing_values() {
    for profiles in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let work = temp.path().join("work");
        let _isolation = scoped_config_home(home.clone());
        let settings = |prefix: &str| {
            if profiles {
                let entries: serde_json::Map<_, _> = (0..60)
                    .map(|n| {
                        (
                            format!("{prefix}-{n}"),
                            json!({"revision":1,"settings":{
                                "envs":{"MARKER":"x".repeat(7168)}
                            }}),
                        )
                    })
                    .collect();
                json!({"agent":{"runtime":{"profiles":entries}}})
            } else {
                let envs = std::collections::BTreeMap::from([(prefix, "x".repeat(40000))]);
                json!({"agent":{"codex":{"envs":envs}}})
            }
        };
        write(&home.join(".opencoder/config.json"), &settings("global"));
        write(&work.join("opencoder.json"), &settings("project"));
        let error = Config::load_with_home(&work, Some(&home))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(if profiles {
                "dispatch budget"
            } else {
                "64 KiB"
            }),
            "{error}"
        );
        assert!(!error.contains(&"x".repeat(32)), "{error}");
    }
}
