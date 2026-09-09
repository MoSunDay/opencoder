use opencoder_core::{
    harness::{self, CodexSettings, RunnerSettings, RuntimeSettings, Versioned},
    Config,
};
use serde_json::json;

#[tokio::test]
async fn named_profiles_survive_config_reload_and_spawned_driver_scope() {
    let root = tempfile::tempdir().unwrap();
    let _home = opencoder_core::scoped_config_home(root.path().into());
    let mut runtime = RuntimeSettings::default();
    runtime.profiles.insert(
        "business".into(),
        Versioned {
            revision: 7,
            settings: serde_json::from_value(json!({"model":"model-pinned","auth_slot":2}))
                .unwrap(),
        },
    );
    let expected = runtime.clone();
    harness::scope::with_execution(None, runtime, async {
        let workdir = root.path().to_owned();
        let task = tokio::spawn(harness::scope::with_settings(
            harness::scope::current(),
            async move { Config::load(&workdir).unwrap().agent.runtime },
        ));
        assert_eq!(task.await.unwrap(), expected);
    })
    .await;
    assert!(harness::scope::current_runtime().is_none());
}

#[test]
fn runner_and_profile_validation_rejects_unpinned_or_invalid_execution_settings() {
    let mut value = json!({"command":["/usr/bin/node","/opt/workflow/cli.js"],"workdir":"/opt/workflow",
        "files":{"/opt/workflow/cli.js":"a".repeat(64)},"parent_unit":"opencoder-agent.service"});
    let parse = |v| serde_json::from_value::<RunnerSettings>(v).unwrap();
    assert!(parse(value.clone()).validate().is_ok());
    value["parent_unit"] = json!("invalid service");
    assert!(parse(value.clone()).validate().is_err());
    value["parent_unit"] = json!(null);
    value["files"] = json!({});
    assert!(parse(value.clone()).validate().is_err());
    assert!(
        serde_json::from_value::<CodexSettings>(json!({"auth_slot":0}))
            .unwrap()
            .validate()
            .is_err()
    );
    let config = Config::default();
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("act")).unwrap();
    std::fs::write(
        root.path().join("act/meta.json"),
        r#"{"name":"act","harness":"codex","harness_profile":"missing"}"#,
    )
    .unwrap();
    opencoder_core::agent::scope::with_root_sync(Some(root.path().into()), || {
        assert!(harness::agent_settings(&config, "act")
            .unwrap_err()
            .contains("missing"));
    });
}
