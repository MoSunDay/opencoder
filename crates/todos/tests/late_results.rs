//! M1 regression: a FAILED execution result that lands after the TODO's
//! status moved on (sibling acceptance rewound the milestone and invalidated
//! it mid-batch) must be discarded — routing it into execution_failed trips
//! the Running guard's `require` and suspends the whole workflow. Mirrors
//! `rewound_sibling_discards_late_successful_result` in interrupt.rs, kept in
//! its own file so interrupt.rs does not keep growing.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use opencoder_core::Config;
use opencoder_llm::{ChatStream, LlmEvent};
use opencoder_store::{LibsqlStore, Store};
use opencoder_todos::{types::*, Runtime};
use tokio_util::sync::CancellationToken;

fn done(text: &str) -> Vec<LlmEvent> {
    vec![LlmEvent::Completed {
        text: text.into(),
        tool_calls: Vec::new(),
        usage: None,
    }]
}

fn dispatch(todo_id: &str, context_mode: &str) -> Vec<LlmEvent> {
    done(&format!(
        r#"{{"operation":"dispatch","todos":[{{"todo_id":"{todo_id}","context_mode":"{context_mode}"}}],"reason":"ready"}}"#
    ))
}

fn candidate_script(summary: &str) -> Vec<LlmEvent> {
    done(&format!(
        r#"{{"status":"candidate","summary":"{summary}","result":"ok","verification":"checked","evidence_refs":[],"recovery_context":{{"summary":"{summary}","refs":[]}}}}"#
    ))
}

fn todo_spec(id: &str, instructions: &str) -> TodoSpec {
    TodoSpec {
        id: id.into(),
        title: id.into(),
        requirement_background: "required".into(),
        instructions: instructions.into(),
        depends_on: vec!["a".into()],
        agent: "act".into(),
        max_attempts: 2,
        acceptance: AcceptanceSpec {
            criteria: "candidate exists".into(),
            required_tool_calls: Vec::new(),
        },
        metadata: serde_json::Value::Null,
    }
}

fn workflow() -> WorkflowSpec {
    WorkflowSpec {
        schema_version: 1,
        id: "wf-late-fail".into(),
        name: "late-fail".into(),
        objective: "finish three items".into(),
        constraints: Vec::new(),
        todos: vec![
            {
                let mut milestone = todo_spec("a", "a instructions");
                milestone.depends_on = Vec::new();
                milestone
            },
            todo_spec("c", "c instructions"),
            todo_spec("b", "hold-me-late-descendant"),
        ],
        metadata: serde_json::Value::Null,
    }
}

/// Scripted ChatStream that holds ONE distinguished call open until released
/// and then ends the stream EMPTY — the released child call fails with "no
/// final candidate", exercising the late-Err path (what MockChatClient's
/// push_hang cannot shape, since its release must still complete normally).
struct ParkingFailClient {
    queue: std::sync::Mutex<std::collections::VecDeque<Vec<LlmEvent>>>,
    park_marker: String,
    notify: Arc<tokio::sync::Notify>,
}

impl ChatStream for ParkingFailClient {
    fn chat_stream(
        &self,
        req: opencoder_llm::ChatRequest,
    ) -> anyhow::Result<tokio::sync::mpsc::Receiver<LlmEvent>> {
        let transcript = req
            .messages
            .iter()
            .filter_map(|message| message["content"].as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let (tx, rx) = tokio::sync::mpsc::channel(128);
        if transcript.contains(&self.park_marker) {
            let notify = self.notify.clone();
            tokio::spawn(async move {
                notify.notified().await;
                drop(tx); // empty stream: the run ends without an assistant reply
            });
        } else {
            let Some(events) = self.queue.lock().unwrap().pop_front() else {
                return Err(anyhow::anyhow!(
                    "parking client exhausted: no script queued"
                ));
            };
            tokio::spawn(async move {
                for event in events {
                    if tx.send(event).await.is_err() {
                        break;
                    }
                }
            });
        }
        Ok(rx)
    }

    fn backend(&self) -> &'static str {
        "parking-fail-mock"
    }
}

async fn wait_for_session_message(
    store: &Arc<dyn Store>,
    workflow_id: &str,
    todo_id: &str,
    marker: &str,
) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let session_id = store
            .list_todo_items(workflow_id)
            .await
            .unwrap()
            .into_iter()
            .find(|item| item.todo_id == todo_id)
            .and_then(|item| item.active_session_id);
        if let Some(session_id) = session_id {
            let transcript = store
                .load_messages(&session_id)
                .await
                .unwrap()
                .iter()
                .map(|message| message.text())
                .collect::<String>();
            if transcript.contains(marker) {
                return;
            }
        }
        assert!(
            Instant::now() < deadline,
            "todo {todo_id} never produced {marker}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn rewound_sibling_discards_late_failed_result() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let notify = Arc::new(tokio::sync::Notify::new());
    let client: Arc<dyn ChatStream> = Arc::new(ParkingFailClient {
        queue: std::sync::Mutex::new(std::collections::VecDeque::from(vec![
            dispatch("a", "new"),
            candidate_script("a done"),
            done(r#"{"operation":"accept","reason":"ok","mark_milestone":true}"#),
            done(
                r#"{"operation":"dispatch","todos":[{"todo_id":"c","context_mode":"new"},{"todo_id":"b","context_mode":"new"}],"reason":"parallel"}"#,
            ),
            candidate_script("c done"),
            done(
                r#"{"operation":"rewind","milestone_todo_id":"a","reason":"ground truth drifted"}"#,
            ),
            done(r#"{"operation":"suspend","reason":"park after rewind"}"#),
        ])),
        park_marker: "hold-me-late-descendant".into(),
        notify: notify.clone(),
    });
    let temp = tempfile::tempdir().unwrap();
    let runtime = Runtime {
        store: store.clone(),
        client,
        config: Config::default(),
        workdir: temp.path().to_path_buf(),
        debug_root: None,
        cancel: CancellationToken::new(),
    };
    let spawned = {
        let runtime = Arc::new(runtime);
        let workflow = workflow();
        tokio::spawn(async move {
            runtime
                .run_new_with_id(workflow, "run-late-fail".into())
                .await
        })
    };

    // Let the sibling c finish its candidate first; only then release b's
    // held call, so b's FAILED result lands after the rewind invalidated it.
    wait_for_session_message(&store, "run-late-fail", "c", "c done").await;
    notify.notify_one();

    let outcome = tokio::time::timeout(Duration::from_secs(10), spawned)
        .await
        .expect("run task finished after the late failure")
        .unwrap();
    let finished = outcome.expect("a discarded failure must not fail the drive loop");
    assert_eq!(finished.status, WorkflowStatus::Suspended);
    assert_eq!(
        finished.terminal_reason.as_deref(),
        Some("park after rewind")
    );
    assert_eq!(finished.todos["a"].status, TodoStatus::Recovering);
    assert_eq!(finished.todos["b"].status, TodoStatus::Invalidated);
    assert_eq!(finished.todos["c"].status, TodoStatus::Invalidated);

    let records = store.todo_events_after("run-late-fail", 0).await.unwrap();
    assert!(
        !records
            .iter()
            .any(|event| event.kind == "todo_execution_failed" && event.payload["todo_id"] == "b"),
        "the invalidated descendant's late failure must be discarded"
    );
    assert!(
        !records.iter().any(|event| event.kind == "runtime_error"),
        "the discarded late failure must not suspend the round"
    );
    assert!(records.iter().any(|event| event.kind == "workflow_rewound"));
}
