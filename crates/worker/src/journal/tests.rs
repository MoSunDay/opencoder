use super::*;

#[test]
fn dag_recovery_reads_without_closing_before_container_cleanup_and_rejects_unpinned_runs() {
    let directory = tempfile::tempdir().unwrap();
    let layout = DirectoryLayout::new(directory.path().into(), None).unwrap();
    let path = layout
        .record_path(ExecutionKind::Dag, "dag-recovery")
        .unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut record = serde_json::json!({"assignment":{
        "index":{"id":"dag-recovery","kind":"dag","created_at":1,"node_id":"node-a","status":"running"},
        "request":{"id":"dag-recovery","kind":"dag","input":{}}},
        "annotations":{"dag_parent":directory.path().join("runs")},"result":null,"error":null,"events":[]});
    std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    let original = std::fs::read(&path).unwrap();
    let loaded = Journal::load(layout.clone()).unwrap();
    assert_eq!(
        loaded.records["dag-recovery"].assignment.index.status,
        ExecutionStatus::Running
    );
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert_eq!(
        loaded.recover().unwrap().records["dag-recovery"]
            .assignment
            .index
            .status,
        ExecutionStatus::Interrupted
    );
    record["annotations"] = serde_json::json!({});
    std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    let original = std::fs::read(&path).unwrap();
    assert!(Journal::load(layout.clone())
        .err()
        .unwrap()
        .to_string()
        .contains("DAG migration blocked"));
    assert_eq!(std::fs::read(&path).unwrap(), original);
    record["assignment"]["index"]["status"] = serde_json::json!("done");
    std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    assert_eq!(Journal::load(layout).unwrap().records.len(), 1);
}

#[test]
fn legacy_journal_kind_comes_from_accepted_request() {
    let mut value = json!({"assignment":{"index":{"id":"team-old","created_at":1,"node_id":"node-a","status":"done"},"request":{"id":"team-old","kind":"team","input":null}},"result":null,"error":null,"events":[]});
    normalize_legacy_index(&mut value).unwrap();
    assert_eq!(value["assignment"]["index"]["kind"], "team");
}

#[test]
fn current_journal_kind_is_not_overwritten() {
    let mut value = json!({"assignment":{"index":{"id":"team-old","created_at":1,"kind":"agent","node_id":"node-a","status":"done"},"request":{"id":"team-old","kind":"team","input":null}}});
    normalize_legacy_index(&mut value).unwrap();
    assert_eq!(value["assignment"]["index"]["kind"], "agent");
}

#[test]
fn current_layout_never_defaults_missing_wire_kind() {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let layout = DirectoryLayout::new(root, None).unwrap();
    let path = layout
        .record_path(ExecutionKind::Agent, "agent-current")
        .unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        serde_json::to_vec(&json!({"assignment":{"index":{"id":"agent-current","created_at":1,"node_id":"node-a","status":"done"},"request":{"id":"agent-current","kind":"agent","input":null}},"result":null,"error":null,"events":[]})).unwrap(),
    )
    .unwrap();

    let error = Journal::open(layout).err().unwrap().to_string();
    assert!(error.contains("missing field `kind`"), "{error}");
}

#[test]
fn legacy_brain_upgrade_guard_preserves_pending_data_and_allows_history() {
    for (kind, input) in [
        (ExecutionKind::Brain, json!({"schema_version":1})),
        (
            ExecutionKind::Agent,
            json!({"_brain":{"run_id":"brain-old"}}),
        ),
        (ExecutionKind::Dag, json!({"brain_receipt":{}})),
        (ExecutionKind::Todos, json!({"playbook_receipt":{}})),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let layout = DirectoryLayout::new(dir.path().to_path_buf(), None).unwrap();
        let path = layout.record_path(kind, "legacy-brain-child").unwrap();
        let mut value = json!({"assignment":{"index":{"id":"legacy-brain-child","kind":kind,"created_at":1,"node_id":"node-a","status":"running"},"request":{"id":"legacy-brain-child","kind":kind,"input":input}},"result":null,"error":null,"events":[]});
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let original = serde_json::to_vec(&value).unwrap();
        std::fs::write(&path, &original).unwrap();
        let error = Journal::open(layout.clone()).err().unwrap().to_string();
        assert!(
            error.contains("migration blocked") && error.contains("legacy-brain-child"),
            "{error}"
        );
        assert_eq!(std::fs::read(&path).unwrap(), original);
        value["assignment"]["index"]["status"] = json!("done");
        let original = serde_json::to_vec(&value).unwrap();
        std::fs::write(&path, &original).unwrap();
        assert_eq!(Journal::open(layout).unwrap().records.len(), 1);
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }
}
