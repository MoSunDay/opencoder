use opencoder_core::{
    fleet::{
        ExecutionCursor, ExecutionIndex, ExecutionKind, ExecutionStatus, MessageCursor,
        MESSAGE_CHUNK_BYTES,
    },
    Message,
};
use opencoder_store::{
    fleet::FleetStore, EventKind, LibsqlStore, ProjectStore, ProjectTodoRecord, ProjectTodoRunKind,
    ProjectTodoRunRecord, ProjectTodoRunStatus, ProjectTodoStatus, SessionEventRecord, SessionMeta,
    Store, TodoEventRecord, TodoItemRecord, TodoWorkflowRecord,
};

fn index(id: &str, created_at: i64) -> ExecutionIndex {
    ExecutionIndex {
        id: id.into(),
        created_at,
        kind: ExecutionKind::Agent,
        node_id: "node-a".into(),
        status: ExecutionStatus::Idle,
    }
}

#[tokio::test]
async fn fleet_keyset_is_stable_across_equal_timestamps() {
    let store = FleetStore::open_memory().await.unwrap();
    for row in [
        index("agent-c", 9),
        index("agent-a", 10),
        index("agent-b", 10),
    ] {
        store.put_index(&row).await.unwrap();
    }
    let first = store.indexes_page(None, None, None, 2).await.unwrap();
    assert_eq!(
        first
            .executions
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        ["agent-a", "agent-b"]
    );
    assert_eq!(
        first.next_cursor,
        Some(ExecutionCursor {
            created_at: 10,
            id: "agent-b".into()
        })
    );
    let second = store
        .indexes_page(None, None, first.next_cursor.as_ref(), 2)
        .await
        .unwrap();
    assert_eq!(
        second
            .executions
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        ["agent-c"]
    );
    assert!(second.next_cursor.is_none());
}

fn session(id: &str) -> SessionMeta {
    SessionMeta {
        id: id.into(),
        title: None,
        agent: None,
        model: None,
        autopilot_mode: None,
        workdir_hash: None,
        created_at: 1,
        updated_at: 1,
        summary: None,
        summary_seq: None,
        summary_images: vec![],
        handoff_seq: None,
        handoff_plan: None,
        skill: None,
        task_type: None,
        requirement: None,
    }
}

#[tokio::test]
async fn huge_utf8_message_is_reassembled_from_bounded_sql_chunks() {
    let dir = tempfile::tempdir().unwrap();
    let store = LibsqlStore::open(dir.path().join("runtime.db"))
        .await
        .unwrap();
    store.create_session(&session("session-a")).await.unwrap();
    let message = Message::user("message-a", "你".repeat(400_000));
    store.append_message("session-a", &message).await.unwrap();
    let expected = serde_json::to_vec(&message.blocks).unwrap();

    let mut cursor = MessageCursor::default();
    let mut actual = Vec::new();
    loop {
        let page = store
            .load_message_page(
                "session-a",
                cursor,
                MESSAGE_CHUNK_BYTES,
                MESSAGE_CHUNK_BYTES * 2,
            )
            .await
            .unwrap();
        assert!(page
            .chunks
            .iter()
            .all(|chunk| chunk.bytes.len() <= MESSAGE_CHUNK_BYTES));
        for chunk in page.chunks {
            actual.extend_from_slice(&chunk.bytes);
        }
        let Some(next) = page.next_cursor else { break };
        assert!(next.seq > cursor.seq || next.offset > cursor.offset);
        cursor = next;
    }
    assert_eq!(actual, expected);
    let decoded: serde_json::Value = serde_json::from_slice(&actual).unwrap();
    assert_eq!(
        decoded[0]["text"].as_str().unwrap().chars().count(),
        400_000
    );
}

#[tokio::test]
async fn event_page_obeys_row_and_payload_budgets() {
    let dir = tempfile::tempdir().unwrap();
    let store = LibsqlStore::open(dir.path().join("runtime.db"))
        .await
        .unwrap();
    store
        .create_session(&session("session-events"))
        .await
        .unwrap();
    for n in 0..3 {
        store
            .append_event(&SessionEventRecord {
                session_id: "session-events".into(),
                kind: EventKind::Step,
                payload: serde_json::json!({"n":n,"text":"x".repeat(32_000)}),
                ts: n,
                seq: None,
                sse_kind: Some("step".into()),
            })
            .await
            .unwrap();
    }
    let first = store
        .events_page("session-events", 0, 2, 40_000)
        .await
        .unwrap();
    assert_eq!(first.events.len(), 1);
    assert!(first.more);
    let next = first.events[0].seq.unwrap();
    assert_eq!(
        store
            .events_page("session-events", next, 2, 80_000)
            .await
            .unwrap()
            .events
            .len(),
        2
    );
    let oversized = store
        .events_page("session-events", 0, 2, 100)
        .await
        .unwrap();
    assert_eq!(oversized.events.len(), 1);
    assert_eq!(oversized.events[0].seq, Some(1));
    assert_eq!(oversized.events[0].payload["omitted"], true);
    assert_eq!(oversized.events[0].payload["read_via"], "event_payload");
    assert!(oversized.more);
    let chunk = store
        .event_payload_chunk("session-events", 1, 0, 64 * 1024)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(chunk.bytes.len() as u64, chunk.total_bytes);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&chunk.bytes).unwrap(),
        serde_json::json!({"n":0,"text":"x".repeat(32_000)})
    );
    let advanced = store
        .events_page("session-events", 1, 2, 40_000)
        .await
        .unwrap();
    assert_eq!(advanced.events[0].seq, Some(2));
}

#[tokio::test]
async fn todo_items_and_project_runs_use_bounded_keyset_pages() {
    let dir = tempfile::tempdir().unwrap();
    let store = LibsqlStore::open(dir.path().join("runtime.db"))
        .await
        .unwrap();
    store.create_session(&session("parent")).await.unwrap();
    let workflow = TodoWorkflowRecord {
        id: "todos-page".into(),
        parent_session_id: "parent".into(),
        status: "running".into(),
        spec_json: serde_json::Value::String("s".repeat(65_535)),
        state_json: serde_json::Value::String("t".repeat(65_534)),
        generation: 1,
        created_at: 1,
        updated_at: 1,
        terminal_reason: None,
    };
    let items = (0..150)
        .map(|ordinal| TodoItemRecord {
            workflow_id: workflow.id.clone(),
            todo_id: format!("item-{ordinal}"),
            ordinal,
            status: "pending".into(),
            attempt: 0,
            active_session_id: None,
            session_history: if ordinal == 0 {
                vec!["x".repeat(70_000)]
            } else {
                vec![]
            },
            result_json: None,
            last_error: None,
            updated_at: 1,
        })
        .collect::<Vec<_>>();
    store
        .create_todo_workflow(
            &workflow,
            &items,
            &TodoEventRecord {
                seq: None,
                workflow_id: workflow.id.clone(),
                kind: "created".into(),
                payload: serde_json::json!({}),
                ts: 1,
            },
        )
        .await
        .unwrap();
    let detail = store
        .get_todo_workflow_detail(&workflow.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(detail.spec_json["omitted"], true);
    assert_eq!(detail.spec_json["total_bytes"], 65_537);
    assert_eq!(detail.state_json.as_str().unwrap().len(), 65_534);
    let spec = store
        .todo_workflow_field_chunk(&workflow.id, "spec_json", 0, 64 * 1024)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(spec.bytes.len(), 64 * 1024);
    assert_eq!(spec.total_bytes, 65_537);
    let first = store
        .list_todo_items_page(&workflow.id, None, 100)
        .await
        .unwrap();
    assert_eq!(first.items.len(), 100);
    assert_eq!(first.next_ordinal, Some(99));
    assert_eq!(first.items[0].session_history["omitted"], true);
    assert_eq!(
        first.items[0].session_history["field"],
        "todo.item.item-0.session_history"
    );
    let mut history = Vec::new();
    let mut offset = 0;
    loop {
        let chunk = store
            .todo_item_field_chunk(&workflow.id, "item-0", "session_history", offset, 64 * 1024)
            .await
            .unwrap()
            .unwrap();
        history.extend_from_slice(&chunk.bytes);
        offset += chunk.bytes.len() as u64;
        if offset == chunk.total_bytes {
            break;
        }
    }
    assert_eq!(
        serde_json::from_slice::<Vec<String>>(&history).unwrap(),
        vec!["x".repeat(70_000)]
    );
    let second = store
        .list_todo_items_page(&workflow.id, first.next_ordinal, 100)
        .await
        .unwrap();
    assert_eq!(second.items.len(), 50);
    assert!(second.next_ordinal.is_none());

    store
        .create_todo(&ProjectTodoRecord {
            id: "project-item".into(),
            milestone_id: None,
            title: "project item".into(),
            draft: "d".repeat(65_537),
            plan_md: Some("p".repeat(65_536)),
            status: ProjectTodoStatus::Draft,
            agent: "act".into(),
            active_session_id: None,
            created_at: 1,
            updated_at: 1,
        })
        .await
        .unwrap();
    let todo = serde_json::to_value(
        store
            .get_todo_summary("project-item")
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(todo["draft"]["omitted"], true);
    assert_eq!(todo["plan_md"].as_str().unwrap().len(), 65_536);
    for version in 1..=25 {
        store
            .create_todo_run(&ProjectTodoRunRecord {
                id: format!("run-{version}"),
                todo_id: "project-item".into(),
                kind: ProjectTodoRunKind::Execute,
                version,
                plan_md: None,
                output_md: (version == 25).then(|| "x".repeat(70_000)),
                agent: "act".into(),
                session_id: Some(format!("agent-run-{version}")),
                status: ProjectTodoRunStatus::Done,
                started_at: version,
                finished_at: Some(version),
                created_at: version,
            })
            .await
            .unwrap();
    }
    let runs = store
        .list_todo_runs_page("project-item", None, 20)
        .await
        .unwrap();
    assert_eq!(runs.runs.len(), 20);
    assert_eq!(runs.next_version, Some(6));
    assert_eq!(
        serde_json::to_value(&runs.runs[0]).unwrap()["output_md"]["omitted"],
        true
    );
    assert_eq!(
        serde_json::to_value(&runs.runs[0]).unwrap()["output_md"]["field"],
        "project.run.run-25.output_md"
    );
    let run = store.get_todo_run_summary("run-25").await.unwrap().unwrap();
    assert_eq!(
        serde_json::to_value(run).unwrap()["output_md"]["omitted"],
        true
    );
    let mut output = Vec::new();
    let mut offset = 0;
    loop {
        let chunk = store
            .project_text_chunk(
                "run",
                "project-item",
                "run-25",
                "output_md",
                offset,
                64 * 1024,
            )
            .await
            .unwrap()
            .unwrap();
        output.extend_from_slice(&chunk.bytes);
        offset += chunk.bytes.len() as u64;
        if offset == chunk.total_bytes {
            break;
        }
    }
    assert_eq!(output, vec![b'x'; 70_000]);
    let last = store
        .list_todo_runs_page("project-item", runs.next_version, 20)
        .await
        .unwrap();
    assert_eq!(last.runs.len(), 5);
    assert!(last.next_version.is_none());
}
