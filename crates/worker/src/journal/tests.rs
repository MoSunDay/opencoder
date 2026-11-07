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
        loaded
            .get_sync("dag-recovery")
            .unwrap()
            .unwrap()
            .assignment
            .index
            .status,
        ExecutionStatus::Running
    );
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert_eq!(
        loaded
            .recover()
            .unwrap()
            .get_sync("dag-recovery")
            .unwrap()
            .unwrap()
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
    assert_eq!(Journal::load(layout).unwrap().summaries.len(), 1);
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
        assert_eq!(Journal::open(layout).unwrap().summaries.len(), 1);
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }
}

fn brain_record(id: &str, status: &str) -> Record {
    serde_json::from_value(json!({"assignment":{
        "index":{"id":id,"kind":"brain","created_at":1,"node_id":"node-test","status":status},
        "request":{"id":id,"kind":"brain","input":{"schema_version":7}}},
        "annotations":{},"result":{"phase":"waiting"},"error":"kept for correction","events":[]
    }))
    .unwrap()
}

#[tokio::test]
async fn brain_control_moves_only_the_status_and_appends_one_event() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal =
        Journal::load(DirectoryLayout::new(dir.path().into(), None).unwrap()).unwrap();
    journal.save(brain_record("brain-pause", "idle")).unwrap();

    let paused = journal
        .brain_control("brain-pause", BrainControl::Pause)
        .await
        .unwrap();
    assert_eq!(paused, ExecutionStatus::Paused);
    let after_pause = journal.get("brain-pause").await.unwrap().unwrap();
    assert_eq!(after_pause.assignment.index.status, ExecutionStatus::Paused);
    assert_eq!(after_pause.result["phase"], "waiting");
    assert_eq!(after_pause.error.as_deref(), Some("kept for correction"));
    assert_eq!(after_pause.events.len(), 1);
    assert_eq!(after_pause.events[0].kind, "status");

    // Repeating the command is a no-op: no second status event.
    assert_eq!(
        journal
            .brain_control("brain-pause", BrainControl::Pause)
            .await
            .unwrap(),
        ExecutionStatus::Paused
    );
    assert_eq!(
        journal
            .get("brain-pause")
            .await
            .unwrap()
            .unwrap()
            .events
            .len(),
        1
    );

    let resumed = journal
        .brain_control("brain-pause", BrainControl::Resume)
        .await
        .unwrap();
    assert_eq!(resumed, ExecutionStatus::Idle);
    let after_resume = journal.get("brain-pause").await.unwrap().unwrap();
    assert_eq!(after_resume.assignment.index.status, ExecutionStatus::Idle);
    assert_eq!(after_resume.error.as_deref(), Some("kept for correction"));

    // A terminal record never moves, and non-brain kinds are rejected.
    journal.save(brain_record("brain-done", "done")).unwrap();
    assert_eq!(
        journal
            .brain_control("brain-done", BrainControl::Pause)
            .await
            .unwrap(),
        ExecutionStatus::Done
    );
    assert!(journal
        .get("brain-done")
        .await
        .unwrap()
        .unwrap()
        .events
        .is_empty());
    let mut agent = brain_record("agent-pause", "idle");
    agent.assignment.index.kind = ExecutionKind::Agent;
    journal.save(agent).unwrap();
    assert!(journal
        .brain_control("agent-pause", BrainControl::Pause)
        .await
        .is_err());
}
