use super::*;

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
