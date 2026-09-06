use opencoder_core::fleet::ExecutionKind;
use opencoder_worker::{migrate_layout, Worker, WorkerOptions};
use serde_json::{json, Value};
use std::path::Path;

fn legacy_record(id: &str, kind: ExecutionKind) -> Value {
    json!({
        "assignment": {
            "index": {
                "id": id,
                "created_at": 7,
                "node_id": "node-old",
                "status": "done"
            },
            "request": {"id": id, "kind": kind, "input": {"prompt":"snapshot"}},
            "definition": null
        },
        "result": null,
        "error": null,
        "events": []
    })
}

fn current_record(id: &str, kind: ExecutionKind) -> Value {
    let mut value = legacy_record(id, kind);
    value["assignment"]["index"]["kind"] = serde_json::to_value(kind).unwrap();
    value
}

fn seed_record(root: &Path, id: &str, kind: ExecutionKind) {
    std::fs::create_dir_all(root.join("executions")).unwrap();
    std::fs::write(
        root.join("executions").join(format!("{id}.json")),
        serde_json::to_vec(&legacy_record(id, kind)).unwrap(),
    )
    .unwrap();
}

fn worker_options(root: &Path) -> WorkerOptions {
    let workdir = root.join("work");
    std::fs::create_dir_all(&workdir).unwrap();
    WorkerOptions {
        name: "migration-test".into(),
        workdir,
        data_dir: root.to_path_buf(),
        workflow_root: None,
        max_runs: Some(1),
        dag: true,
    }
}

#[tokio::test]
async fn migration_copies_and_verifies_typed_tree_while_retaining_legacy() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("node");
    seed_record(&root, "dag-old", ExecutionKind::Dag);
    std::fs::create_dir_all(root.join("resources/dag-old/prompts")).unwrap();
    std::fs::write(root.join("resources/dag-old/prompts/pinned"), "resource").unwrap();
    std::fs::create_dir_all(root.join("workflow/dag-old/first")).unwrap();
    std::fs::create_dir_all(root.join("workflow/rootfs/usr")).unwrap();
    std::fs::write(root.join("workflow/dag-old/input.json"), "{}").unwrap();
    std::fs::write(root.join("workflow/dag-old/first/output.txt"), "artifact").unwrap();
    std::fs::write(root.join("workflow/rootfs/usr/python"), "runtime").unwrap();

    let report = migrate_layout(&root, None).unwrap();
    assert_eq!(report.migrated, 1);
    let current = root.join("dag/dag-old");
    assert!(current.join("execution.json").is_file());
    assert!(current.join("migration.json").is_file());
    assert_eq!(
        std::fs::read_to_string(current.join("resources/prompts/pinned")).unwrap(),
        "resource"
    );
    assert_eq!(
        std::fs::read_to_string(current.join("first/output.txt")).unwrap(),
        "artifact"
    );
    assert!(root.join("executions/dag-old.json").is_file());
    assert!(root.join("workflow/dag-old/first/output.txt").is_file());
    assert!(root.join("workflow/rootfs/usr/python").is_file());
    assert!(!root.join("dag/rootfs").exists());

    let mut evolved: Value =
        serde_json::from_slice(&std::fs::read(current.join("execution.json")).unwrap()).unwrap();
    evolved["result"] = json!({"new":"state"});
    std::fs::write(
        current.join("execution.json"),
        serde_json::to_vec(&evolved).unwrap(),
    )
    .unwrap();
    assert_eq!(migrate_layout(&root, None).unwrap().already_current, 1);
    let worker = Worker::open(worker_options(&root), None).await.unwrap();
    drop(worker);

    let old_path = root.join("executions/dag-old.json");
    let mut changed: Value = serde_json::from_slice(&std::fs::read(&old_path).unwrap()).unwrap();
    changed["result"] = json!({"changed":"legacy"});
    std::fs::write(&old_path, serde_json::to_vec(&changed).unwrap()).unwrap();
    let error = Worker::open(worker_options(&root), None)
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("conflicting legacy and current"), "{error}");
}

#[test]
fn migration_conflict_is_fail_closed_and_preserves_both_records() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("node");
    seed_record(&root, "agent-old", ExecutionKind::Agent);
    let current = root.join("agent/agent-old");
    std::fs::create_dir_all(&current).unwrap();
    let mut different = legacy_record("agent-old", ExecutionKind::Agent);
    different["assignment"]["index"]["kind"] = json!("agent");
    different["result"] = json!({"conflict":true});
    std::fs::write(
        current.join("execution.json"),
        serde_json::to_vec(&different).unwrap(),
    )
    .unwrap();

    let error = migrate_layout(&root, None).unwrap_err().to_string();
    assert!(error.contains("conflicts with legacy"), "{error}");
    assert!(root.join("executions/agent-old.json").is_file());
    assert!(current.join("execution.json").is_file());
    assert!(!current.join("migration.json").exists());
}

#[test]
fn migration_rejects_existing_unowned_destination_before_publishing_anything() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("node");
    seed_record(&root, "agent-a", ExecutionKind::Agent);
    seed_record(&root, "agent-z", ExecutionKind::Agent);
    let unowned = root.join("agent/agent-z");
    std::fs::create_dir_all(&unowned).unwrap();
    std::fs::write(unowned.join("unknown"), "do not adopt").unwrap();

    let error = migrate_layout(&root, None).unwrap_err().to_string();
    assert!(error.contains("has no execution record"), "{error}");
    assert!(!root.join("agent/agent-a").exists());
    assert_eq!(
        std::fs::read_to_string(unowned.join("unknown")).unwrap(),
        "do not adopt"
    );
}

#[cfg(unix)]
#[test]
fn migration_symlink_failure_never_publishes_partial_destination() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("node");
    seed_record(&root, "agent-a", ExecutionKind::Agent);
    seed_record(&root, "agent-z", ExecutionKind::Agent);
    let outside = dir.path().join("outside");
    std::fs::create_dir_all(root.join("resources/agent-z")).unwrap();
    std::fs::write(&outside, "outside").unwrap();
    std::os::unix::fs::symlink(&outside, root.join("resources/agent-z/link")).unwrap();

    let error = migrate_layout(&root, None).unwrap_err().to_string();
    assert!(error.contains("refuses symlink"), "{error}");
    assert!(root.join("executions/agent-a.json").is_file());
    assert!(root.join("executions/agent-z.json").is_file());
    assert!(!root.join("agent/agent-a").exists());
    assert!(!root.join("agent/agent-z").exists());
    assert!(!root.join("agent").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn current_execution_record_symlink_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("node");
    let current = root.join("agent/agent-current");
    let outside = dir.path().join("outside.json");
    std::fs::create_dir_all(&current).unwrap();
    std::fs::write(
        &outside,
        serde_json::to_vec(&current_record("agent-current", ExecutionKind::Agent)).unwrap(),
    )
    .unwrap();
    std::os::unix::fs::symlink(&outside, current.join("execution.json")).unwrap();

    let error = Worker::open(worker_options(&root), None)
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("path contains a symlink"), "{error}");
    assert!(outside.is_file());
}

#[cfg(unix)]
#[test]
fn migration_rejects_symlinked_receipt_and_destination_trees() {
    for entry in ["migration.json", "resources", "team"] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("node");
        let kind = if entry == "team" {
            ExecutionKind::Team
        } else {
            ExecutionKind::Agent
        };
        let id = format!("{}-old", kind.prefix());
        seed_record(&root, &id, kind);
        let current = root.join(kind.prefix()).join(&id);
        std::fs::create_dir_all(&current).unwrap();
        std::fs::write(
            current.join("execution.json"),
            serde_json::to_vec(&current_record(&id, kind)).unwrap(),
        )
        .unwrap();
        let outside = dir.path().join(format!("outside-{entry}"));
        if entry == "migration.json" {
            std::fs::write(&outside, r#"{"version":1,"legacy_execution_sha256":"x"}"#).unwrap();
        } else {
            std::fs::create_dir_all(root.join(entry).join(&id)).unwrap();
            std::fs::write(root.join(entry).join(&id).join("pinned"), "same").unwrap();
            std::fs::create_dir_all(&outside).unwrap();
            std::fs::write(outside.join("pinned"), "same").unwrap();
        }
        std::os::unix::fs::symlink(&outside, current.join(entry)).unwrap();

        let error = migrate_layout(&root, None).unwrap_err().to_string();
        assert!(error.contains("symlink"), "{entry}: {error}");
        assert!(root.join("executions").join(format!("{id}.json")).is_file());
        assert!(outside.exists());
    }
}

#[cfg(unix)]
#[tokio::test]
async fn node_ownership_files_cannot_be_symlinks() {
    for name in [
        "node.lock",
        "node-id",
        "runtime.db",
        "runtime.db-wal",
        "runtime.db-shm",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("node");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            &outside,
            if name == "node-id" {
                "node-external"
            } else {
                "keep"
            },
        )
        .unwrap();
        std::os::unix::fs::symlink(&outside, root.join(name)).unwrap();

        let error = Worker::open(worker_options(&root), None)
            .await
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("symlink"), "{name}: {error}");
        assert!(outside.is_file());
    }
}

#[cfg(unix)]
#[tokio::test]
async fn startup_rejects_kind_root_symlink_before_owned_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("node");
    let outside = dir.path().join("outside");
    std::fs::create_dir_all(outside.join("bundles")).unwrap();
    std::fs::write(outside.join("keep"), "safe").unwrap();
    std::fs::create_dir_all(&root).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("dag")).unwrap();

    let error = Worker::open(worker_options(&root), None)
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("path contains a symlink"), "{error}");
    assert_eq!(
        std::fs::read_to_string(outside.join("keep")).unwrap(),
        "safe"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn startup_rejects_legacy_workflow_ancestor_symlink_before_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("node");
    let outside = dir.path().join("outside");
    let fake_state = outside.join("legacy/bundles/run/step/runc-state/fake-owned");
    std::fs::create_dir_all(&fake_state).unwrap();
    std::fs::write(fake_state.join("keep"), "safe").unwrap();
    let configured = dir.path().join("configured");
    std::fs::create_dir_all(&configured).unwrap();
    std::os::unix::fs::symlink(&outside, configured.join("link")).unwrap();
    let mut options = worker_options(&root);
    options.workflow_root = Some(configured.join("link/legacy"));

    let error = Worker::open(options, None).await.err().unwrap().to_string();
    assert!(
        error.contains("legacy workflow root contains a symlink"),
        "{error}"
    );
    assert_eq!(
        std::fs::read_to_string(fake_state.join("keep")).unwrap(),
        "safe"
    );
}

#[cfg(unix)]
#[test]
fn migration_lock_and_external_workflow_root_cannot_be_symlinks() {
    let lock_case = tempfile::tempdir().unwrap();
    let lock_root = lock_case.path().join("node");
    let outside_lock = lock_case.path().join("outside-lock");
    std::fs::create_dir_all(&lock_root).unwrap();
    std::fs::write(&outside_lock, "keep").unwrap();
    std::os::unix::fs::symlink(&outside_lock, lock_root.join("node.lock")).unwrap();
    let error = migrate_layout(&lock_root, None).unwrap_err().to_string();
    assert!(error.contains("symlink"), "{error}");
    assert_eq!(std::fs::read_to_string(outside_lock).unwrap(), "keep");

    let workflow_case = tempfile::tempdir().unwrap();
    let root = workflow_case.path().join("node");
    seed_record(&root, "dag-old", ExecutionKind::Dag);
    let outside_workflow = workflow_case.path().join("outside-workflow");
    let workflow_link = workflow_case.path().join("workflow-link");
    std::fs::create_dir_all(&outside_workflow).unwrap();
    std::os::unix::fs::symlink(&outside_workflow, &workflow_link).unwrap();
    let error = migrate_layout(&root, Some(&workflow_link))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("legacy workflow root") && error.contains("symlink"),
        "{error}"
    );
    assert!(root.join("executions/dag-old.json").is_file());
}
