//! Interrupt and concurrency-conflict recovery for the TODO runtime: a held
//! in-flight LLM call (MockChatClient::push_hang) is interrupted either by an
//! external store-level interrupt, a local cancellation, or an external
//! generation bump — the run must stop, persist a non-running workflow, and
//! stay resumable.

use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use opencoder_core::Config;
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_store::{LibsqlStore, Store};
use opencoder_todos::{types::*, Runtime};
use tokio_util::sync::CancellationToken;

const CANDIDATE: &str = r#"{"status":"candidate","summary":"done","result":"ok","verification":"checked","evidence_refs":[],"recovery_context":{"summary":"done","refs":[]}}"#;

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

fn spec() -> WorkflowSpec {
    WorkflowSpec {
        schema_version: 1,
        id: "wf-test".into(),
        name: "test".into(),
        objective: "finish one item".into(),
        constraints: Vec::new(),
        todos: vec![TodoSpec {
            id: "step-1".into(),
            title: "step".into(),
            requirement_background: "required by test".into(),
            instructions: "return the candidate".into(),
            depends_on: Vec::new(),
            agent: "act".into(),
            max_attempts: 2,
            allowed_tools: vec![],
            acceptance: AcceptanceSpec {
                criteria: "candidate exists".into(),
                required_tool_calls: Vec::new(),
            },
            metadata: serde_json::Value::Null,
        }],
        metadata: serde_json::Value::Null,
    }
}

fn runtime(store: &Arc<dyn Store>, client: Arc<dyn ChatStream>, dir: &Path) -> Runtime {
    Runtime {
        store: store.clone(),
        client,
        config: Config::default(),
        workdir: dir.to_path_buf(),
        debug_root: None,
        cancel: CancellationToken::new(),
    }
}

async fn load(store: &Arc<dyn Store>, id: &str) -> WorkflowState {
    opencoder_todos::persistence::load(store, id)
        .await
        .unwrap()
        .expect("workflow must be persisted")
        .1
}

async fn kinds(store: &Arc<dyn Store>, id: &str) -> Vec<String> {
    store
        .todo_events_after(id, 0)
        .await
        .unwrap()
        .into_iter()
        .map(|event| event.kind)
        .collect()
}

/// Poll the durable item projections until the TODO shows as running (the
/// batch committed "todos_dispatched" and execution is in flight).
async fn wait_until_running(store: &Arc<dyn Store>, workflow_id: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let items = store.list_todo_items(workflow_id).await.unwrap();
        if items.iter().any(|item| item.status == "running") {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "todo in {workflow_id} never reached running"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn external_interrupt_cancels_inflight_todo_and_is_resumable() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let hang = Arc::new(tokio::sync::Notify::new());
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(dispatch("step-1", "new"))
            .push_hang(hang.clone()),
    );
    let temp = tempfile::tempdir().unwrap();
    let run_runtime = Arc::new(runtime(&store, mock.clone(), temp.path()));
    let spawned = {
        let rt = run_runtime.clone();
        tokio::spawn(async move { rt.run_new_with_id(spec(), "run-interrupt".into()).await })
    };

    wait_until_running(&store, "run-interrupt").await;
    opencoder_todos::interrupt(&store, "run-interrupt", "test interrupt")
        .await
        .unwrap();
    hang.notify_one();

    // The run must stop promptly — Ok or a persisted Err, never hung or
    // silently continuing.
    let outcome = tokio::time::timeout(Duration::from_secs(10), spawned)
        .await
        .expect("run task finished after interrupt")
        .unwrap();
    let state = load(&store, "run-interrupt").await;
    assert_eq!(state.status, WorkflowStatus::Suspended);
    assert_ne!(
        state.todos["step-1"].status,
        TodoStatus::Running,
        "in-flight TODO must be torn down"
    );
    assert!(
        state.todos["step-1"].active_session_id.is_some(),
        "child session kept for resume"
    );
    let events = kinds(&store, "run-interrupt").await;
    assert!(events.contains(&"workflow_interrupted".into()));
    if let Ok(finished) = &outcome {
        assert_eq!(finished.status, WorkflowStatus::Suspended);
    }

    // A fresh process (fresh mock, same store) resumes to completion.
    let mock2 = Arc::new(
        MockChatClient::new()
            .push_script(dispatch("step-1", "resume"))
            .push_script(done(CANDIDATE))
            .push_script(done(
                r#"{"operation":"accept","reason":"ok","mark_milestone":false}"#,
            ))
            .push_script(done(r#"{"operation":"complete","reason":"all passed"}"#)),
    );
    let resumed = runtime(&store, mock2, temp.path())
        .resume("run-interrupt")
        .await
        .unwrap();

    assert_eq!(resumed.status, WorkflowStatus::Completed);
    assert_eq!(resumed.todos["step-1"].status, TodoStatus::Passed);
    assert_eq!(resumed.todos["step-1"].attempt, 2);
}

#[tokio::test]
async fn local_cancel_mid_todo_marks_item_interrupted_and_stops_cleanly() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let hang = Arc::new(tokio::sync::Notify::new());
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(dispatch("step-1", "new"))
            .push_hang(hang.clone()),
    );
    let temp = tempfile::tempdir().unwrap();
    let cancel = CancellationToken::new();
    let mut run_config = runtime(&store, mock, temp.path());
    run_config.cancel = cancel.clone();
    let run_runtime = Arc::new(run_config);
    let spawned = {
        let rt = run_runtime.clone();
        tokio::spawn(async move { rt.run_new_with_id(spec(), "run-cancel".into()).await })
    };

    wait_until_running(&store, "run-cancel").await;
    cancel.cancel();
    hang.notify_one();

    let state = tokio::time::timeout(Duration::from_secs(10), spawned)
        .await
        .expect("run task finished after local cancel")
        .unwrap()
        .expect("local cancel resolves to Ok(suspended state)");

    assert_eq!(state.status, WorkflowStatus::Suspended);
    assert_eq!(state.todos["step-1"].status, TodoStatus::Interrupted);
    let events = store.todo_events_after("run-cancel", 0).await.unwrap();
    let failed = events
        .iter()
        .find(|event| event.kind == "todo_execution_failed")
        .expect("per-item failure is recorded");
    assert_eq!(failed.payload["todo_id"], "step-1");
    assert_eq!(
        failed.payload["interrupted"], true,
        "cancellation maps the item to Interrupted, not Failed"
    );
    assert!(events
        .iter()
        .any(|event| event.kind == "workflow_interrupted"));
}

#[tokio::test]
async fn generation_conflict_stops_the_run() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let hang = Arc::new(tokio::sync::Notify::new());
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(dispatch("step-1", "new"))
            .push_hang(hang.clone()),
    );
    let temp = tempfile::tempdir().unwrap();
    let run_runtime = Arc::new(runtime(&store, mock.clone(), temp.path()));
    let spawned = {
        let rt = run_runtime.clone();
        tokio::spawn(async move { rt.run_new_with_id(spec(), "run-conflict".into()).await })
    };

    wait_until_running(&store, "run-conflict").await;
    // An external writer bumps the generation behind the runtime's back.
    let (workflow, mut state) = opencoder_todos::persistence::load(&store, "run-conflict")
        .await
        .unwrap()
        .unwrap();
    state.generation += 1;
    opencoder_todos::persistence::commit(
        &store,
        &workflow,
        &state,
        "external_change",
        serde_json::json!({}),
    )
    .await
    .unwrap();
    hang.notify_one();

    // The run must observe the conflict and stop instead of steamrolling it.
    let outcome = tokio::time::timeout(Duration::from_secs(10), spawned)
        .await
        .expect("run task finished after generation conflict")
        .unwrap();
    let final_state = load(&store, "run-conflict").await;
    assert_ne!(
        final_state.status,
        WorkflowStatus::Running,
        "workflow must not be left running unattended"
    );
    let events = kinds(&store, "run-conflict").await;
    assert!(events.contains(&"external_change".into()));
    if outcome.is_ok() {
        assert!(
            events.iter().any(|kind| matches!(
                kind.as_str(),
                "workflow_suspended" | "runtime_error" | "workflow_interrupted"
            )),
            "an Ok outcome must have persisted a suspension"
        );
    }
    assert_eq!(
        mock.call_count(),
        2,
        "no further LLM scheduling after the conflict"
    );
}

#[tokio::test]
async fn interrupt_rejects_terminal_workflow() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(dispatch("step-1", "new"))
            .push_script(done(CANDIDATE))
            .push_script(done(
                r#"{"operation":"accept","reason":"ok","mark_milestone":false}"#,
            ))
            .push_script(done(r#"{"operation":"complete","reason":"all passed"}"#)),
    );
    let temp = tempfile::tempdir().unwrap();
    let runtime = runtime(&store, mock, temp.path());

    let state = runtime
        .run_new_with_id(spec(), "run-done".into())
        .await
        .unwrap();
    assert_eq!(state.status, WorkflowStatus::Completed);

    let error = opencoder_todos::interrupt(&store, "run-done", "too late")
        .await
        .unwrap_err();

    assert!(format!("{error:#}").contains("cannot interrupt terminal workflow"));
}

/// Bug #5 regression: with max_attempts=1, an external store-level interrupt
/// that lands while the local cancel token has not been flipped yet (the
/// `poll_interrupt` 250ms window) must NOT be mistaken for a plain execution
/// failure — the TODO must stay Interrupted (not Failed) and the externally
/// persisted Suspended state must survive untouched.
#[tokio::test]
async fn external_interrupt_window_keeps_max_attempt_todo_unfailed() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let hang = Arc::new(tokio::sync::Notify::new());
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(dispatch("step-1", "new"))
            .push_hang(hang.clone()),
    );
    let temp = tempfile::tempdir().unwrap();
    // A runtime whose local cancel token is never touched: only the external
    // store write interrupts the run, exercising the polling window.
    let run_runtime = Arc::new(runtime(&store, mock, temp.path()));
    let spawned = {
        let rt = run_runtime.clone();
        tokio::spawn(async move {
            let mut workflow = spec();
            workflow.todos[0].max_attempts = 1;
            rt.run_new_with_id(workflow, "run-window".into()).await
        })
    };

    wait_until_running(&store, "run-window").await;
    opencoder_todos::interrupt(&store, "run-window", "external stop")
        .await
        .unwrap();
    // Release the in-flight call immediately — the child session fails while
    // the local token is still unset, the exact misclassification window.
    hang.notify_one();

    let outcome = tokio::time::timeout(Duration::from_secs(10), spawned)
        .await
        .expect("run task finished after external interrupt window")
        .unwrap();
    if let Ok(finished) = &outcome {
        assert_eq!(finished.status, WorkflowStatus::Suspended);
    }

    // The externally written Suspended verdict must be intact: not Failed,
    // not re-committed from the stale local Running state.
    let state = load(&store, "run-window").await;
    assert_eq!(state.status, WorkflowStatus::Suspended);
    assert_eq!(state.terminal_reason.as_deref(), Some("external stop"));
    assert_ne!(
        state.todos["step-1"].status,
        TodoStatus::Failed,
        "a max_attempts=1 TODO must not fail from an external interrupt"
    );
    assert_eq!(
        state.todos["step-1"].status,
        TodoStatus::Interrupted,
        "the external suspension verdict for the in-flight TODO must survive"
    );
    assert_eq!(state.todos["step-1"].attempt, 1);
    assert!(
        state.todos["step-1"].active_session_id.is_some(),
        "child session kept for resume"
    );

    // The local execution-failure bookkeeping must never have been applied.
    let events = kinds(&store, "run-window").await;
    assert!(
        !events.iter().any(|kind| kind == "todo_execution_failed"),
        "the external interrupt must not be recorded as a local execution failure"
    );
    assert!(events.contains(&"workflow_interrupted".into()));
}

/// Bug #5 regression (runner half): when a local cancel races an external
/// store write, `drive_inner`'s interrupt branch must adopt the externally
/// persisted state instead of committing a local "workflow_interrupted"
/// (with "local interrupt requested") over it.
#[tokio::test]
async fn local_cancel_after_external_write_adopts_external_state() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let hang = Arc::new(tokio::sync::Notify::new());
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(dispatch("step-1", "new"))
            .push_hang(hang.clone()),
    );
    let temp = tempfile::tempdir().unwrap();
    let cancel = CancellationToken::new();
    let mut run_config = runtime(&store, mock, temp.path());
    run_config.cancel = cancel.clone();
    let run_runtime = Arc::new(run_config);
    let spawned = {
        let rt = run_runtime.clone();
        tokio::spawn(async move { rt.run_new_with_id(spec(), "run-mixed".into()).await })
    };

    wait_until_running(&store, "run-mixed").await;
    // External verdict first, then a local cancel while the batch is still
    // holding the in-flight call.
    opencoder_todos::interrupt(&store, "run-mixed", "external stop")
        .await
        .unwrap();
    cancel.cancel();
    hang.notify_one();

    let outcome = tokio::time::timeout(Duration::from_secs(10), spawned)
        .await
        .expect("run task finished after mixed interrupt")
        .unwrap()
        .expect("mixed interrupt resolves to the adopted external state");

    assert_eq!(outcome.status, WorkflowStatus::Suspended);
    assert_eq!(outcome.terminal_reason.as_deref(), Some("external stop"));

    // The store keeps the external verdict; the runtime's local interrupt
    // commit must not have overwritten it.
    let state = load(&store, "run-mixed").await;
    assert_eq!(state.status, WorkflowStatus::Suspended);
    assert_eq!(state.terminal_reason.as_deref(), Some("external stop"));
    assert_eq!(state.todos["step-1"].status, TodoStatus::Interrupted);
    assert_ne!(state.todos["step-1"].status, TodoStatus::Failed);
    assert_eq!(state.todos["step-1"].attempt, 1);

    let events = kinds(&store, "run-mixed").await;
    assert!(
        !events.iter().any(|kind| kind == "todo_execution_failed"),
        "no local execution-failure bookkeeping over the external interrupt"
    );
    let interrupts: Vec<_> = store
        .todo_events_after("run-mixed", 0)
        .await
        .unwrap()
        .into_iter()
        .filter(|event| event.kind == "workflow_interrupted")
        .collect();
    assert_eq!(
        interrupts.len(),
        1,
        "exactly the external workflow_interrupted commit must exist"
    );
    assert_eq!(interrupts[0].payload["reason"], "external stop");
}

// ---------------------------------------------------------------------------
// Bug #16a: a successful TODO result that lands after the todo's status has
// moved on (external interrupt, or a sibling acceptance rewinding a
// milestone) must be discarded instead of tripping `candidate`'s Running
// guard, suspending the round, or clobbering the external verdict.
// ---------------------------------------------------------------------------

/// Scripted ChatStream that can hold ONE distinguished call open until
/// released and then still complete it successfully — the piece
/// `MockChatClient::push_hang` cannot do (its release ends the stream
/// empty, which fails the execution). Calls whose transcript contains
/// `park_marker` park on `notify`; every other call pops the FIFO queue and
/// an empty queue fails the call like an exhausted mock.
struct ParkingChatClient {
    queue: std::sync::Mutex<std::collections::VecDeque<Vec<LlmEvent>>>,
    parked_events: Vec<LlmEvent>,
    park_marker: String,
    notify: Arc<tokio::sync::Notify>,
}

impl ChatStream for ParkingChatClient {
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
            let events = self.parked_events.clone();
            tokio::spawn(async move {
                notify.notified().await;
                for event in events {
                    if tx.send(event).await.is_err() {
                        break;
                    }
                }
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
        "parking-mock"
    }
}

fn candidate_script(summary: &str) -> Vec<LlmEvent> {
    done(&format!(
        r#"{{"status":"candidate","summary":"{summary}","result":"ok","verification":"checked","evidence_refs":[],"recovery_context":{{"summary":"{summary}","refs":[]}}}}"#
    ))
}

/// Wait until the given TODO's child session has produced an assistant
/// message containing `marker` — i.e. the sibling's execution has finished
/// recording its candidate while the parked TODO is still in flight.
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
            "TODO {todo_id} never produced the marker message"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Bug #16a regression (external half): the child session SUCCEEDS after an
/// external store-level interrupt has already persisted Suspended and marked
/// the in-flight TODO Interrupted. The successful result must be discarded —
/// no `todo_candidate_ready` commit over the external verdict, no
/// runtime_error suspension — and drive must adopt the external state.
#[tokio::test]
async fn external_interrupt_after_successful_todo_discards_result_cleanly() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let notify = Arc::new(tokio::sync::Notify::new());
    let mut workflow = spec();
    workflow.todos[0].instructions = "hold-me-then-succeed".into();
    let client: Arc<dyn ChatStream> = Arc::new(ParkingChatClient {
        queue: std::sync::Mutex::new(std::collections::VecDeque::from(vec![dispatch(
            "step-1", "new",
        )])),
        parked_events: done(CANDIDATE),
        park_marker: "hold-me-then-succeed".into(),
        notify: notify.clone(),
    });
    let temp = tempfile::tempdir().unwrap();
    let run_runtime = Arc::new(runtime(&store, client, temp.path()));
    let spawned = {
        let rt = run_runtime.clone();
        tokio::spawn(async move {
            rt.run_new_with_id(workflow, "run-ok-interrupt".into())
                .await
        })
    };

    wait_until_running(&store, "run-ok-interrupt").await;
    opencoder_todos::interrupt(&store, "run-ok-interrupt", "external stop")
        .await
        .unwrap();
    // Release the held call: the child session finishes SUCCESSFULLY after
    // the external verdict is already durable.
    notify.notify_one();

    let outcome = tokio::time::timeout(Duration::from_secs(10), spawned)
        .await
        .expect("run task finished after discarding the late result")
        .unwrap();
    let finished = outcome.expect("a discarded result must not fail the drive loop");
    assert_eq!(finished.status, WorkflowStatus::Suspended);
    assert_eq!(finished.terminal_reason.as_deref(), Some("external stop"));

    // The externally written verdict stays intact.
    let state = load(&store, "run-ok-interrupt").await;
    assert_eq!(state.status, WorkflowStatus::Suspended);
    assert_eq!(state.terminal_reason.as_deref(), Some("external stop"));
    assert_eq!(state.todos["step-1"].status, TodoStatus::Interrupted);
    assert_eq!(state.todos["step-1"].attempt, 1);

    let events = kinds(&store, "run-ok-interrupt").await;
    assert!(
        !events.iter().any(|kind| kind == "todo_candidate_ready"),
        "the superseded result must not be committed as a candidate"
    );
    assert!(
        !events.iter().any(|kind| kind == "runtime_error"),
        "discarding the result must not suspend the round with a runtime error"
    );
}

/// Bug #16a regression (local half): when a sibling acceptance rewinds the
/// milestone mid-batch, the descendant still holding an in-flight execution
/// is Invalidated; its late successful result must be discarded instead of
/// tripping `candidate`'s Running guard and failing the whole round.
#[tokio::test]
async fn rewound_sibling_discards_late_successful_result() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let notify = Arc::new(tokio::sync::Notify::new());
    let workflow = WorkflowSpec {
        schema_version: 1,
        id: "wf-test".into(),
        name: "test".into(),
        objective: "finish three items".into(),
        constraints: Vec::new(),
        todos: vec![
            TodoSpec {
                id: "a".into(),
                title: "milestone".into(),
                requirement_background: "required".into(),
                instructions: "a instructions".into(),
                depends_on: Vec::new(),
                agent: "act".into(),
                max_attempts: 2,
                allowed_tools: vec![],
                acceptance: AcceptanceSpec {
                    criteria: "candidate exists".into(),
                    required_tool_calls: Vec::new(),
                },
                metadata: serde_json::Value::Null,
            },
            TodoSpec {
                id: "c".into(),
                title: "sibling".into(),
                requirement_background: "required".into(),
                instructions: "c instructions".into(),
                depends_on: vec!["a".into()],
                agent: "act".into(),
                max_attempts: 2,
                allowed_tools: vec![],
                acceptance: AcceptanceSpec {
                    criteria: "candidate exists".into(),
                    required_tool_calls: Vec::new(),
                },
                metadata: serde_json::Value::Null,
            },
            TodoSpec {
                id: "b".into(),
                title: "late descendant".into(),
                requirement_background: "required".into(),
                instructions: "hold-me-late-descendant".into(),
                depends_on: vec!["a".into()],
                agent: "act".into(),
                max_attempts: 2,
                allowed_tools: vec![],
                acceptance: AcceptanceSpec {
                    criteria: "candidate exists".into(),
                    required_tool_calls: Vec::new(),
                },
                metadata: serde_json::Value::Null,
            },
        ],
        metadata: serde_json::Value::Null,
    };
    let dispatch_both = done(
        r#"{"operation":"dispatch","todos":[{"todo_id":"c","context_mode":"new"},{"todo_id":"b","context_mode":"new"}],"reason":"parallel"}"#,
    );
    let client: Arc<dyn ChatStream> = Arc::new(ParkingChatClient {
        queue: std::sync::Mutex::new(std::collections::VecDeque::from(vec![
            dispatch("a", "new"),
            candidate_script("a done"),
            done(r#"{"operation":"accept","reason":"ok","mark_milestone":true}"#),
            dispatch_both,
            candidate_script("c done"),
            done(
                r#"{"operation":"rewind","milestone_todo_id":"a","reason":"ground truth drifted"}"#,
            ),
            done(r#"{"operation":"suspend","reason":"park after rewind"}"#),
        ])),
        parked_events: candidate_script("b done"),
        park_marker: "hold-me-late-descendant".into(),
        notify: notify.clone(),
    });
    let temp = tempfile::tempdir().unwrap();
    let run_runtime = Arc::new(runtime(&store, client, temp.path()));
    let spawned = {
        let rt = run_runtime.clone();
        tokio::spawn(async move {
            rt.run_new_with_id(workflow, "run-rewind-discard".into())
                .await
        })
    };

    // Let the sibling c finish its candidate first; only then release b's
    // held call, so b's successful result lands after the rewind.
    wait_for_session_message(&store, "run-rewind-discard", "c", "c done").await;
    notify.notify_one();

    let outcome = tokio::time::timeout(Duration::from_secs(10), spawned)
        .await
        .expect("run task finished after the rewind discard")
        .unwrap();
    let finished = outcome.expect("a discarded result must not fail the drive loop");
    assert_eq!(finished.status, WorkflowStatus::Suspended);
    assert_eq!(
        finished.terminal_reason.as_deref(),
        Some("park after rewind")
    );
    assert_eq!(finished.todos["a"].status, TodoStatus::Recovering);
    assert_eq!(finished.todos["b"].status, TodoStatus::Invalidated);
    assert_eq!(finished.todos["c"].status, TodoStatus::Invalidated);

    // Exactly two candidates entered the state machine: the milestone "a"
    // and the sibling that got accepted; the invalidated descendant's late
    // result never became a candidate and the round did not blow up.
    let records = store
        .todo_events_after("run-rewind-discard", 0)
        .await
        .unwrap();
    let candidate_ids: std::collections::HashSet<&str> = records
        .iter()
        .filter(|event| event.kind == "todo_candidate_ready")
        .map(|event| event.payload["todo_id"].as_str().unwrap())
        .collect();
    assert_eq!(candidate_ids.len(), 2);
    assert!(candidate_ids.contains("a"));
    assert!(
        candidate_ids.contains("b") ^ candidate_ids.contains("c"),
        "exactly one of the two batched descendants may become a candidate; \
         the invalidated one's late result must be discarded"
    );
    let events = kinds(&store, "run-rewind-discard").await;
    assert!(events.contains(&"workflow_rewound".into()));
    assert!(
        !events.iter().any(|kind| kind == "runtime_error"),
        "the discarded late result must not suspend the round"
    );
}
