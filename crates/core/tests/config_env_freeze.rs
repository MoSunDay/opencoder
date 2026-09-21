//! Env-freeze contract for versioned operator executions: the frozen view
//! ignores env overlays ("snapshot is final"); the live view keeps applying
//! them. Deliberately a standalone test binary — it mutates process env and
//! must not race sibling tests that install `scoped_config_home`.

use opencoder_core::Config;

#[test]
fn frozen_load_skips_env_overlays() {
    let workdir = tempfile::tempdir().unwrap();
    let execution = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(execution.path().join(".opencoder")).unwrap();
    std::fs::write(
        execution.path().join(".opencoder/config.json"),
        serde_json::json!({"model": "frozen/snapshot-model"}).to_string(),
    )
    .unwrap();

    std::env::set_var("OPENCODER_MODEL", "env/overlay-model");
    let live = Config::load_with_home(workdir.path(), Some(execution.path())).unwrap();
    let frozen = Config::load_with_home_frozen(workdir.path(), Some(execution.path())).unwrap();
    std::env::remove_var("OPENCODER_MODEL");
    assert_eq!(live.model, "env/overlay-model", "live view follows env");
    assert_eq!(
        frozen.model, "frozen/snapshot-model",
        "the frozen view ignores env entirely"
    );
}
