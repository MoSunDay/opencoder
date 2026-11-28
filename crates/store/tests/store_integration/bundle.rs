//! Binary bundle export/import round-trip contract (incl. subagent links).

use crate::common::conv;
use opencoder_store::{
    export_bundle, Delivery, SessionBundle, SessionInput, SessionMeta, Store, TursoStore,
};
use tempfile::TempDir;

#[tokio::test]
async fn bundle_export_import_roundtrip() {
    use opencoder_store::{
        import_bundle, read_bundle, write_bundle, SubagentStatus, SubagentTaskRecord,
    };

    let dir = TempDir::new().unwrap();
    let store = TursoStore::open(dir.path().join("test.db")).await.unwrap();

    // Create parent session with messages.
    let parent_meta = SessionMeta {
        id: "parent-1".into(),
        title: Some("parent".into()),
        agent: Some("act".into()),
        model: Some("test-model".into()),
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
        kind: None,
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
        kind: None,
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
    let store2 = TursoStore::open(dir2.path().join("test2.db"))
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

// --- Export is a read: orphaned inputs must not depend on a write ---------

fn input(session: &str, id: &str, delivery: Delivery) -> SessionInput {
    SessionInput {
        seq: None,
        id: id.into(),
        session_id: session.into(),
        delivery,
        prompt: format!("p-{id}"),
        images: Vec::new(),
        display_text: None,
        admitted_seq: 0,
        promoted_seq: None,
    }
}

fn exported_ids(bundle: &SessionBundle) -> Vec<String> {
    bundle.inputs.iter().map(|input| input.id.clone()).collect()
}

/// One session holding every input state an export has to tell apart:
/// `done-1`/`done-2` are promoted AND recorded (consumed into the transcript,
/// never exported), `orphan` is promoted but unrecorded (invisible to
/// `pending_inputs`, still owed to the next drain) and `queued` is pending.
async fn session_with_input_states(store: &TursoStore) {
    crate::common::make_session(store, "inputs-1", 1).await;
    for (id, delivery) in [
        ("done-1", Delivery::Steer),
        ("orphan", Delivery::Steer),
        ("done-2", Delivery::Queue),
        ("queued", Delivery::Queue),
    ] {
        store
            .admit_input(&input("inputs-1", id, delivery))
            .await
            .unwrap();
    }
    // Steer: promote both, consume only `done-1` -> `orphan` stays promoted.
    let promoted = store
        .promote_inputs("inputs-1", 2, Delivery::Steer)
        .await
        .unwrap();
    assert_eq!(promoted.len(), 2, "both steer rows get promoted");
    store
        .mark_inputs_recorded("inputs-1", &[promoted[0]])
        .await
        .unwrap();
    // Queue: promote only `done-2` (admitted_seq 3) and consume it;
    // `queued` (admitted_seq 4) stays pending.
    let promoted = store
        .promote_inputs("inputs-1", 3, Delivery::Queue)
        .await
        .unwrap();
    assert_eq!(promoted.len(), 1, "only the first queue row gets promoted");
    store
        .mark_inputs_recorded("inputs-1", &promoted)
        .await
        .unwrap();
}

/// Consume the fixture's only orphan, leaving the session with no orphan row.
async fn consume_the_only_orphan(store: &TursoStore) {
    assert_eq!(store.recover_orphan_inputs("inputs-1").await.unwrap(), 1);
    let pending = store
        .pending_inputs("inputs-1", Delivery::Steer)
        .await
        .unwrap();
    assert_eq!(pending.len(), 1, "the recovered orphan is pending again");
    let promoted = store
        .promote_inputs("inputs-1", pending[0].admitted_seq, Delivery::Steer)
        .await
        .unwrap();
    store
        .mark_inputs_recorded("inputs-1", &promoted)
        .await
        .unwrap();
}

/// Exporting through a reader beside the live writer must carry the orphaned
/// (promoted-but-unrecorded) row: `pending_inputs` cannot see it, and the
/// recovery write a reader would need is refused. Pre-fix this is red — the
/// refusal was ignored and the row silently vanished from the bundle.
#[tokio::test]
async fn reader_export_beside_a_live_writer_carries_orphaned_inputs_without_writing() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.db");
    let writer = TursoStore::open(&path).await.unwrap();
    session_with_input_states(&writer).await;

    // The reader exports first, while the orphan is still promoted: nothing
    // has recovered it, so only a read of the orphan rows can carry it.
    let reader = TursoStore::open_reader(&path).await.unwrap();
    let from_reader = export_bundle(&reader, "inputs-1").await.unwrap();
    assert_eq!(
        exported_ids(&from_reader),
        ["orphan", "queued"],
        "a reader must export the orphaned row: the pending queue row plus the orphan, consumed rows excluded"
    );
    assert_eq!(
        exported_ids(&export_bundle(&writer, "inputs-1").await.unwrap()),
        exported_ids(&from_reader),
        "reader and writer must agree on the exported inputs"
    );
    assert!(
        from_reader
            .inputs
            .iter()
            .all(|input| input.promoted_seq.is_none()),
        "exported inputs are re-admitted as pending on import"
    );

    // Export changed nothing: the orphan is still promoted, so the writer's
    // own recovery still finds exactly one row afterwards.
    assert!(writer
        .pending_inputs("inputs-1", Delivery::Steer)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        writer.recover_orphan_inputs("inputs-1").await.unwrap(),
        1,
        "export must not recover the orphan in place"
    );
    // Now that the writer recovered it, both handles still agree.
    assert_eq!(
        exported_ids(&export_bundle(&reader, "inputs-1").await.unwrap()),
        ["orphan", "queued"]
    );
}

/// No orphan data is neither an error nor an omission: a session without
/// inputs exports an empty input list through a reader, a session whose
/// orphans were all consumed exports only what is still pending, and an
/// unknown session is reported instead of exported as empty.
#[tokio::test]
async fn reader_export_without_orphan_data_stays_exact() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.db");
    let writer = TursoStore::open(&path).await.unwrap();
    crate::common::make_session(&writer, "empty-1", 1).await;
    session_with_input_states(&writer).await;
    consume_the_only_orphan(&writer).await;

    let reader = TursoStore::open_reader(&path).await.unwrap();
    let empty = export_bundle(&reader, "empty-1").await.unwrap();
    assert!(empty.inputs.is_empty(), "no inputs, no orphans, no error");

    let consumed = export_bundle(&reader, "inputs-1").await.unwrap();
    assert_eq!(exported_ids(&consumed), ["queued"]);

    let missing = export_bundle(&reader, "absent-1").await.unwrap_err();
    assert!(
        missing.to_string().contains("session not found"),
        "{missing}"
    );
    assert!(reader.orphan_inputs("absent-1").await.unwrap().is_empty());
}
