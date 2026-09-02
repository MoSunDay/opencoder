//! Binary bundle export/import round-trip contract (incl. subagent links).

use crate::common::conv;
use opencoder_store::{LibsqlStore, SessionMeta, Store};
use tempfile::TempDir;

#[tokio::test]
async fn bundle_export_import_roundtrip() {
    use opencoder_store::{
        export_bundle, import_bundle, read_bundle, write_bundle, SubagentStatus, SubagentTaskRecord,
    };

    let dir = TempDir::new().unwrap();
    let store = LibsqlStore::open(dir.path().join("test.db")).await.unwrap();

    // Create parent session with messages.
    let parent_meta = SessionMeta {
        id: "parent-1".into(),
        title: Some("parent".into()),
        agent: Some("act".into()),
        model: Some("test-model".into()),
        autopilot_mode: None,
        workdir_hash: None,
        created_at: 1000,
        updated_at: 2000,
        summary: None,
        summary_seq: None,
        summary_images: vec![],
        handoff_seq: None,
        handoff_plan: None,
        skill: None,
        task_type: None,
        requirement: None,
    };
    store.create_session(&parent_meta).await.unwrap();
    let msgs = conv("parent", 4);
    store.append_messages("parent-1", &msgs).await.unwrap();

    // Create child session with messages.
    let child_meta = SessionMeta {
        id: "child-1".into(),
        title: Some("child".into()),
        agent: Some("explore".into()),
        model: Some("test-model".into()),
        autopilot_mode: None,
        workdir_hash: None,
        created_at: 1100,
        updated_at: 2100,
        summary: None,
        summary_seq: None,
        summary_images: vec![],
        handoff_seq: None,
        handoff_plan: None,
        skill: None,
        task_type: None,
        requirement: None,
    };
    store.create_session(&child_meta).await.unwrap();
    let child_msgs = conv("child", 2);
    store.append_messages("child-1", &child_msgs).await.unwrap();

    // Link parent -> child.
    let task = SubagentTaskRecord {
        task_id: "task-1".into(),
        parent_session_id: "parent-1".into(),
        child_session_id: "child-1".into(),
        parent_message_id: None,
        agent: "explore".into(),
        prompt: "investigate".into(),
        result: Some("done".into()),
        status: SubagentStatus::Completed,
        ok: Some(true),
        started_at: 1500,
        completed_at: Some(1600),
    };
    store.create_subagent_task(&task).await.unwrap();

    // Export.
    let bundle = export_bundle(&store, "parent-1").await.unwrap();
    assert_eq!(bundle.meta.id, "parent-1");
    assert_eq!(bundle.messages.len(), 4);
    assert_eq!(bundle.subagents.len(), 1);
    assert_eq!(bundle.subagents[0].child.meta.id, "child-1");
    assert_eq!(bundle.subagents[0].child.messages.len(), 2);

    // Write to binary, read back.
    let mut buf = Vec::new();
    write_bundle(&bundle, &mut buf).unwrap();
    assert_eq!(&buf[..8], b"OPENCODR");
    let mut cursor = std::io::Cursor::new(&buf);
    let restored = read_bundle(&mut cursor).unwrap();
    assert_eq!(restored.messages.len(), 4);
    assert_eq!(restored.subagents.len(), 1);

    // Import into a fresh store.
    let dir2 = TempDir::new().unwrap();
    let store2 = LibsqlStore::open(dir2.path().join("test2.db"))
        .await
        .unwrap();
    let id = import_bundle(&store2, &restored, None).await.unwrap();
    assert_eq!(id, "parent-1");

    // Verify parent messages.
    let msgs2 = store2.load_messages("parent-1").await.unwrap();
    assert_eq!(msgs2.len(), 4);

    // Verify child session + messages.
    let child2 = store2.load_messages("child-1").await.unwrap();
    assert_eq!(child2.len(), 2);

    // Verify subagent link.
    let tasks2 = store2.list_subagent_tasks("parent-1").await.unwrap();
    assert_eq!(tasks2.len(), 1);
    assert_eq!(tasks2[0].child_session_id, "child-1");

    // Idempotent re-import should be skipped.
    import_bundle(&store2, &restored, None).await.unwrap();
    let msgs3 = store2.load_messages("parent-1").await.unwrap();
    assert_eq!(msgs3.len(), 4, "re-import must not duplicate");
}
