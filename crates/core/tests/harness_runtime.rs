use opencoder_core::{
    harness::{self, CodexSettings, RuntimeSettings, Versioned},
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
fn profile_validation_rejects_invalid_execution_settings() {
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

#[test]
fn historical_runtime_fields_round_trip_without_executable_registrations() {
    let value =
        json!({"profiles":{},"runners":{"old":{"revision":1,"settings":{"command":["/old/bin"]}}}});
    let runtime: RuntimeSettings = serde_json::from_value(value.clone()).unwrap();
    assert!(runtime.profiles.is_empty());
    assert_eq!(serde_json::to_value(runtime).unwrap(), value);
}
