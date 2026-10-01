use super::*;

#[path = "../../../dag-runtime/tests/support/container.rs"]
#[allow(dead_code)]
mod native;

#[tokio::test]
async fn dag_acceptance_pins_config_and_lost_driver_cleanup_precedes_terminal_status() {
    let (directory, deps, store) = test_deps().await;
    seed_todo(&store, "t1", ProjectTodoStatus::Running).await;
    seed_run(&store, "template", "t1", ProjectTodoRunKind::Execute).await;
    let mut record = store.get_todo_run("template").await.unwrap().unwrap();
    record.id = "project-recovery".into();
    record.version = 2;
    record.executor_kind = opencoder_store::ProjectExecutorKind::Dag;
    store.create_todo_run(&record).await.unwrap();
    let fixture = native::ContainerFixture::open(directory.path());
    opencoder_dag_binary::save_binary_version(
        &fixture.pool,
        "program",
        "fixture",
        &std::fs::read("/bin/true").unwrap(),
    )
    .unwrap();
    let mut config = fixture.config.clone();
    config.dag.data_dir = Some(directory.path().join("dag/runs"));
    let spec: opencoder_dag::DagSpec =
        serde_json::from_value(serde_json::json!({"name":"accepted","steps":[
        {"name":"run","kind":{"type":"binary","resource":"program","args":[]}}]}))
        .unwrap();
    let (accepted, run, root, resume) =
        crate::executor::dag_state::prepare(&deps, &record, config, spec, "stable".into())
            .await
            .unwrap();
    assert!(!resume);
    let record = store.get_todo_run(&record.id).await.unwrap().unwrap();
    assert_eq!(record.output_ref.as_deref(), Some(root.to_str().unwrap()));
    let (restored, frozen, restored_root, resume) =
        crate::executor::dag_state::restored(&deps, &record)
            .unwrap()
            .unwrap();
    assert!(resume);
    assert_eq!(restored_root, root);
    assert_eq!(restored.dag.data_dir, accepted.dag.data_dir);
    assert_eq!(frozen.spec, run.spec);
    std::fs::write(
        directory.path().join("opencoder.json"),
        r#"{"dag":{"rootfs_dir":"/missing","data_dir":"/wrong"}}"#,
    )
    .unwrap();
    let container =
        opencoder_dag_runtime::sandbox::run::RunContainer::start(&root, &restored, &frozen)
            .await
            .unwrap();
    std::mem::forget(container);
    assert!(crate::recover::converge_lost_run(&deps, &record.id).await);
    assert_eq!(run_status(&store, &record.id).await, RunStatus::Cancelled);
    assert!(std::fs::read_dir(root.join("runc-state"))
        .unwrap()
        .next()
        .is_none());
    assert!(!std::fs::read_to_string("/proc/self/mountinfo")
        .unwrap()
        .contains(root.to_str().unwrap()));
    assert!(deps.persistence_error.lock().unwrap().is_none());
}
