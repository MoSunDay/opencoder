use opencoder_dag_runtime::resources::freeze;
use serde_json::json;

fn spec(resource: &str) -> opencoder_dag::DagSpec {
    opencoder_dag::decode_spec(&json!({"name":"pins","steps":[{"name":"run","kind":{"type":"binary","resource":resource,"args":[]}}]})).unwrap()
}

#[test]
fn accepted_binary_version_survives_publish_rollback_and_pool_removal() {
    let temp = tempfile::tempdir().unwrap();
    let pool = temp.path().join("pool");
    let first = std::fs::read("/bin/true").unwrap();
    let second = std::fs::read("/bin/false").unwrap();
    opencoder_dag_binary::save_binary_version(&pool, "program", "first", &first).unwrap();
    let mut config = opencoder_core::Config::default();
    config.dag.binary_dir = Some(pool.clone());
    let run = temp.path().join("run-1");
    freeze(&run, &config, &spec("program")).unwrap();
    opencoder_dag_binary::save_binary_version(&pool, "program", "second", &second).unwrap();
    freeze(&temp.path().join("run-2"), &config, &spec("program")).unwrap();
    opencoder_dag_binary::rollback_binary(&pool, "program", 1).unwrap();
    freeze(&temp.path().join("run-3"), &config, &spec("program@v2")).unwrap();
    opencoder_dag_binary::delete_binary(&pool, "program").unwrap();
    freeze(&run, &config, &spec("program")).unwrap();
    assert_eq!(std::fs::read(run.join("run/meta/program")).unwrap(), first);
    assert_eq!(
        std::fs::read(temp.path().join("run-2/run/meta/program")).unwrap(),
        second
    );
    assert_eq!(
        std::fs::read(temp.path().join("run-3/run/meta/program")).unwrap(),
        second
    );
    let receipt: serde_json::Value =
        serde_json::from_slice(&std::fs::read(run.join("resources.json")).unwrap()).unwrap();
    assert_eq!(receipt["run"]["version"], 1);
}

#[test]
fn corrupt_missing_and_changed_resources_fail_without_a_pin_receipt() {
    let temp = tempfile::tempdir().unwrap();
    let pool = temp.path().join("pool");
    let bytes = std::fs::read("/bin/true").unwrap();
    opencoder_dag_binary::save_binary_version(&pool, "program", "valid", &bytes).unwrap();
    let mut config = opencoder_core::Config::default();
    config.dag.binary_dir = Some(pool.clone());
    let missing = temp.path().join("missing");
    assert!(freeze(&missing, &config, &spec("missing")).is_err());
    assert!(!missing.join("resources.json").exists());
    let accepted = temp.path().join("accepted");
    freeze(&accepted, &config, &spec("program")).unwrap();
    std::fs::write(accepted.join("run/meta/program"), "corrupt").unwrap();
    assert!(freeze(&accepted, &config, &spec("program")).is_err());
    assert!(freeze(&accepted, &config, &spec("other")).is_err());
    std::fs::write(
        opencoder_dag_binary::binary_bin(&pool, "program", 1),
        "corrupt",
    )
    .unwrap();
    let corrupt = temp.path().join("corrupt");
    assert!(freeze(&corrupt, &config, &spec("program")).is_err());
    assert!(!corrupt.join("resources.json").exists());
}
