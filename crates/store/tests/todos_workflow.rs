use opencoder_store::{
    LibsqlStore, SessionMeta, Store, TodoEventRecord, TodoItemRecord, TodoWorkflowRecord,
    TASK_TYPE_TODO_WORKFLOW,
};

fn parent(id: &str) -> SessionMeta {
    SessionMeta {
        id: id.into(),
        title: Some("workflow".into()),
        agent: Some("workflow".into()),
        model: Some("test/model".into()),
        created_at: 1,
        updated_at: 1,
        task_type: Some(TASK_TYPE_TODO_WORKFLOW.into()),
        ..Default::default()
    }
}

fn workflow(generation: i64, status: &str) -> TodoWorkflowRecord {
    TodoWorkflowRecord {
        id: "wf-1".into(),
        parent_session_id: "parent-1".into(),
        status: status.into(),
        spec_json: serde_json::json!({"id":"wf-1"}),
        state_json: serde_json::json!({"generation":generation}),
        generation,
        created_at: 1,
        updated_at: generation + 1,
        terminal_reason: None,
    }
}

fn item(status: &str) -> TodoItemRecord {
    TodoItemRecord {
        workflow_id: "wf-1".into(),
        todo_id: "step-1".into(),
        ordinal: 1,
        status: status.into(),
        attempt: 0,
        active_session_id: None,
        session_history: Vec::new(),
        result_json: None,
        last_error: None,
        updated_at: 1,
    }
}

fn event(kind: &str) -> TodoEventRecord {
    TodoEventRecord {
        seq: None,
        workflow_id: "wf-1".into(),
        kind: kind.into(),
        payload: serde_json::json!({}),
        ts: 1,
    }
}

#[tokio::test]
async fn workflow_projection_and_event_commit_are_atomic_and_versioned() {
    let store = LibsqlStore::open_memory().await.unwrap();
    store.create_session(&parent("parent-1")).await.unwrap();
    let seq = store
        .create_todo_workflow(
            &workflow(0, "pending"),
            &[item("pending")],
            &event("created"),
        )
        .await
        .unwrap();
    assert_eq!(seq, 1);

    store
        .commit_todo_transition(
            &workflow(1, "running"),
            &[item("running")],
            &event("started"),
        )
        .await
        .unwrap();
    let got = store.get_todo_workflow("wf-1").await.unwrap().unwrap();
    assert_eq!(got.generation, 1);
    assert_eq!(
        store.list_todo_items("wf-1").await.unwrap()[0].status,
        "running"
    );
    assert_eq!(store.todo_events_after("wf-1", 0).await.unwrap().len(), 2);

    let error = store
        .commit_todo_transition(&workflow(1, "failed"), &[item("failed")], &event("stale"))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("generation conflict"));
    assert_eq!(store.todo_events_after("wf-1", 0).await.unwrap().len(), 2);
}

#[tokio::test]
async fn schema_v8_reopens_at_v9_with_todo_tables() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v8.db");
    {
        let db = libsql::Builder::new_local(&path).build().await.unwrap();
        let conn = db.connect().unwrap();
        conn.execute("CREATE TABLE schema_version (version INTEGER NOT NULL)", ())
            .await
            .unwrap();
        conn.execute("INSERT INTO schema_version VALUES (8)", ())
            .await
            .unwrap();
    }
    let store = LibsqlStore::open(&path).await.unwrap();
    store.create_session(&parent("parent-1")).await.unwrap();
    store
        .create_todo_workflow(
            &workflow(0, "pending"),
            &[item("pending")],
            &event("created"),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn reverse_todo_history_skips_global_sequence_gaps_and_respects_byte_budget() {
    let store = LibsqlStore::open_memory().await.unwrap();
    store.create_session(&parent("parent-1")).await.unwrap();
    store
        .create_todo_workflow(&workflow(0, "running"), &[], &event("created"))
        .await
        .unwrap();
    let mut other = workflow(0, "running");
    other.id = "other".into();
    let mut unrelated = event("noise");
    unrelated.workflow_id = other.id.clone();
    store
        .create_todo_workflow(&other, &[], &unrelated)
        .await
        .unwrap();
    for generation in 1..=300 {
        other.generation = generation;
        store
            .commit_todo_transition(&other, &[], &unrelated)
            .await
            .unwrap();
    }
    let mut large = event("dispatch");
    large.payload = serde_json::json!({"text":"中".repeat(30_000)});
    let dispatch = store
        .commit_todo_transition(&workflow(1, "running"), &[], &large)
        .await
        .unwrap();
    let head = store
        .commit_todo_transition(&workflow(2, "done"), &[], &event("completed"))
        .await
        .unwrap();
    let page = store
        .todo_events_before("wf-1", head + 1, 200, 2048)
        .await
        .unwrap();
    assert!(page.more);
    assert_eq!(
        page.events
            .iter()
            .map(|e| e.seq.unwrap())
            .collect::<Vec<_>>(),
        vec![head]
    );
    let page = store
        .todo_events_before("wf-1", head, 200, 2048)
        .await
        .unwrap();
    assert!(page.more);
    assert_eq!(page.events[0].seq, Some(dispatch));
    assert_eq!(page.events[0].payload["omitted"], true);
    let chunk = store
        .todo_event_payload_chunk("wf-1", dispatch, 0, 64 * 1024)
        .await
        .unwrap()
        .unwrap();
    assert!(chunk.total_bytes > 64 * 1024);
    let page = store
        .todo_events_before("wf-1", dispatch, 200, 2048)
        .await
        .unwrap();
    assert!(!page.more);
    assert_eq!(
        page.events
            .iter()
            .map(|e| e.seq.unwrap())
            .collect::<Vec<_>>(),
        vec![1]
    );
    let page = store
        .todo_events_before("wf-1", head + 1, 2, 200_000)
        .await
        .unwrap();
    assert!(page.more);
    assert_eq!(
        page.events
            .iter()
            .map(|e| e.seq.unwrap())
            .collect::<Vec<_>>(),
        vec![head, dispatch]
    );
}
