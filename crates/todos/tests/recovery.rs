//! Crash-recovery and terminal-decision integration tests for the durable
//! TODO workflow runtime. An in-process "kill -9 during acceptance" is
//! simulated by exhausting the mock LLM mid-run; persistence must hold a
//! suspended workflow whose resume self-heals the acceptance boundary.

use std::{path::Path, sync::Arc};

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

fn make_runtime(store: &Arc<dyn Store>, client: Arc<dyn ChatStream>, dir: &Path) -> Runtime {
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

#[tokio::test]
async fn acceptance_crash_then_resume_self_heals() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    // The third parent call (acceptance verdict) finds the mock exhausted,
    // simulating a process dying between "todo_acceptance_started" and the
    // verdict — the crash window WP recovery must survive.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(dispatch("step-1", "new"))
            .push_script(done(CANDIDATE)),
    );
    let temp = tempfile::tempdir().unwrap();
    let runtime = make_runtime(&store, mock.clone(), temp.path());

    let crashed = runtime.run_new_with_id(spec(), "run-crash".into()).await;

    assert!(
        crashed.is_err(),
        "fatal error during acceptance must surface"
    );
    let suspended = load(&store, "run-crash").await;
    assert_eq!(suspended.status, WorkflowStatus::Suspended);
    // Suspension rolls every mid-flight todo (incl. Accepting) back to
    // Interrupted — a suspended workflow must not leave items claiming an
    // in-progress status; resume reconciles and re-dispatches from there.
    assert_eq!(suspended.todos["step-1"].status, TodoStatus::Interrupted);
    assert_eq!(suspended.todos["step-1"].attempt, 1);
    let first_session = suspended.todos["step-1"].active_session_id.clone();
    assert!(first_session.is_some(), "dispatched session was committed");
    let events = kinds(&store, "run-crash").await;
    assert_eq!(
        events.last().map(String::as_str),
        Some("runtime_error"),
        "the crash must be recorded as a runtime_error suspension"
    );

    // A fresh process (fresh mock, same store) resumes and self-heals.
    let mock2 = Arc::new(
        MockChatClient::new()
            .push_script(dispatch("step-1", "resume"))
            .push_script(done(CANDIDATE))
            .push_script(done(
                r#"{"operation":"accept","reason":"meets criteria","mark_milestone":false}"#,
            ))
            .push_script(done(r#"{"operation":"complete","reason":"all passed"}"#)),
    );
    let resumed = make_runtime(&store, mock2.clone(), temp.path())
        .resume("run-crash")
        .await
        .unwrap();

    assert_eq!(resumed.status, WorkflowStatus::Completed);
    assert_eq!(resumed.todos["step-1"].status, TodoStatus::Passed);
    assert_eq!(resumed.todos["step-1"].attempt, 2);
    assert_eq!(
        resumed.todos["step-1"].active_session_id, first_session,
        "resume mode reuses the crashed child session"
    );
    assert_eq!(resumed.todos["step-1"].session_history.len(), 1);
    let events = kinds(&store, "run-crash").await;
    assert!(events.contains(&"workflow_resumed".into()));
    assert!(
        events.contains(&"runtime_error".into()),
        "event history is append-only across processes"
    );
    assert_eq!(mock2.call_count(), 4);
}

#[tokio::test]
async fn parent_fail_decision_fails_workflow() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(dispatch("step-1", "new"))
            .push_script(done(CANDIDATE))
            .push_script(done(r#"{"operation":"fail","reason":"bad"}"#))
            .push_script(done(r#"{"operation":"fail","reason":"give up"}"#)),
    );
    let temp = tempfile::tempdir().unwrap();
    let runtime = make_runtime(&store, mock, temp.path());

    let state = runtime
        .run_new_with_id(spec(), "run-fail".into())
        .await
        .unwrap();

    assert_eq!(state.status, WorkflowStatus::Failed);
    assert_eq!(state.todos["step-1"].status, TodoStatus::Failed);
    assert_eq!(state.terminal_reason.as_deref(), Some("give up"));
    let events = kinds(&store, "run-fail").await;
    let todo_at = events
        .iter()
        .position(|kind| kind == "todo_failed")
        .expect("todo_failed recorded");
    let workflow_at = events
        .iter()
        .position(|kind| kind == "workflow_failed")
        .expect("workflow_failed recorded");
    assert!(todo_at < workflow_at, "the item fails before the workflow");
}

#[tokio::test]
async fn parent_suspend_decision_parks_workflow() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(done(r#"{"operation":"suspend","reason":"wait for input"}"#)),
    );
    let temp = tempfile::tempdir().unwrap();
    let runtime = make_runtime(&store, mock.clone(), temp.path());

    let state = runtime
        .run_new_with_id(spec(), "run-park".into())
        .await
        .unwrap();

    assert_eq!(state.status, WorkflowStatus::Suspended);
    assert_eq!(state.terminal_reason.as_deref(), Some("wait for input"));
    assert_eq!(state.todos["step-1"].status, TodoStatus::Pending);
    let events = kinds(&store, "run-park").await;
    assert!(
        !events.iter().any(|kind| kind == "todos_dispatched"),
        "no TODO was dispatched for a park decision"
    );
    assert_eq!(mock.call_count(), 1);
}

#[tokio::test]
async fn persistence_list_returns_summaries_and_honors_limit() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let workflow = spec();
    for id in ["wf-a", "wf-b", "wf-c"] {
        let mut state =
            opencoder_todos::domain::initial_state(&workflow, id.into(), format!("parent-{id}"));
        state.status = WorkflowStatus::Running;
        opencoder_todos::parent::create_session(&store, &state, &Config::default())
            .await
            .unwrap();
        opencoder_todos::persistence::create(&store, &workflow, &state)
            .await
            .unwrap();
    }

    let limited = opencoder_todos::persistence::list(&store, 2).await.unwrap();
    assert_eq!(limited.len(), 2);
    let known = ["wf-a", "wf-b", "wf-c"];
    assert!(limited
        .iter()
        .all(|summary| known.contains(&summary.id.as_str())));
    assert!(limited.iter().all(|summary| summary.status == "running"));

    let full = opencoder_todos::persistence::list(&store, 100)
        .await
        .unwrap();
    for id in known {
        assert!(
            full.iter().any(|summary| summary.id == id),
            "{id} missing from listing"
        );
    }
}

// ---------------------------------------------------------------------------
// Bug #16b: empty/invalid parent decisions are correctable model mistakes.
// The parent gets a bounded correction re-ask instead of a one-shot
// suspension; only a persistently invalid decision suspends the workflow.
// ---------------------------------------------------------------------------

fn two_step_spec() -> WorkflowSpec {
    let mut workflow = spec();
    workflow.todos.push(TodoSpec {
        id: "step-2".into(),
        title: "second".into(),
        requirement_background: "required by test".into(),
        instructions: "return the candidate".into(),
        depends_on: vec!["step-1".into()],
        agent: "act".into(),
        max_attempts: 2,
        allowed_tools: vec![],
        acceptance: AcceptanceSpec {
            criteria: "candidate exists".into(),
            required_tool_calls: Vec::new(),
        },
        metadata: serde_json::Value::Null,
    });
    workflow
}

/// One unparseable JSON reply must be corrected in-session and the workflow
/// must continue instead of suspending on the first model hiccup.
#[tokio::test]
async fn unparseable_parent_decision_is_corrected_without_suspending() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(done("sorry, I cannot produce JSON right now"))
            .push_script(dispatch("step-1", "new"))
            .push_script(done(CANDIDATE))
            .push_script(done(
                r#"{"operation":"accept","reason":"ok","mark_milestone":false}"#,
            ))
            .push_script(done(r#"{"operation":"complete","reason":"all passed"}"#)),
    );
    let temp = tempfile::tempdir().unwrap();
    let runtime = make_runtime(&store, mock.clone(), temp.path());

    let state = runtime
        .run_new_with_id(spec(), "run-correct".into())
        .await
        .unwrap();

    assert_eq!(state.status, WorkflowStatus::Completed);
    assert_eq!(state.todos["step-1"].status, TodoStatus::Passed);
    assert_eq!(mock.call_count(), 5, "exactly one correction re-ask");
    // The correction prompt landed in the parent transcript.
    let parent = store
        .load_messages(&state.parent_session_id)
        .await
        .unwrap()
        .iter()
        .map(|message| message.text())
        .collect::<String>();
    assert!(
        parent.contains("could not be parsed"),
        "the re-ask must explain why the previous reply was rejected"
    );
}

/// A dispatch decision for a non-runnable TODO must be re-asked with a
/// correction instead of suspending the round through dispatch validation.
#[tokio::test]
async fn non_runnable_dispatch_is_corrected_without_suspending() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mock = Arc::new(
        MockChatClient::new()
            // step-2 is not runnable while step-1 is still pending.
            .push_script(dispatch("step-2", "new"))
            .push_script(dispatch("step-1", "new"))
            .push_script(done(CANDIDATE))
            .push_script(done(
                r#"{"operation":"accept","reason":"ok","mark_milestone":false}"#,
            ))
            .push_script(dispatch("step-2", "new"))
            .push_script(done(CANDIDATE))
            .push_script(done(
                r#"{"operation":"accept","reason":"ok","mark_milestone":false}"#,
            ))
            .push_script(done(r#"{"operation":"complete","reason":"all passed"}"#)),
    );
    let temp = tempfile::tempdir().unwrap();
    let runtime = make_runtime(&store, mock.clone(), temp.path());

    let state = runtime
        .run_new_with_id(two_step_spec(), "run-correct-dispatch".into())
        .await
        .unwrap();

    assert_eq!(state.status, WorkflowStatus::Completed);
    assert_eq!(state.todos["step-1"].status, TodoStatus::Passed);
    assert_eq!(state.todos["step-2"].status, TodoStatus::Passed);
    assert_eq!(mock.call_count(), 8, "exactly one correction re-ask");
    let parent = store
        .load_messages(&state.parent_session_id)
        .await
        .unwrap()
        .iter()
        .map(|message| message.text())
        .collect::<String>();
    assert!(
        parent.contains("TODO step-2 is not runnable"),
        "the correction must carry the validation error"
    );
    let events = kinds(&store, "run-correct-dispatch").await;
    assert!(!events.iter().any(|kind| kind == "runtime_error"));
}

/// Three consecutive unparseable replies exhaust the correction budget: the
/// workflow suspends and the underlying parse error stays visible.
#[tokio::test]
async fn three_unparseable_parent_decisions_suspend_with_error_preserved() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(done("nope one"))
            .push_script(done("nope two"))
            .push_script(done("nope three")),
    );
    let temp = tempfile::tempdir().unwrap();
    let runtime = make_runtime(&store, mock.clone(), temp.path());

    let crashed = runtime
        .run_new_with_id(spec(), "run-junk".into())
        .await
        .unwrap_err();

    let reason = format!("{crashed:#}");
    assert!(reason.contains("invalid JSON"), "{reason}");
    let suspended = load(&store, "run-junk").await;
    assert_eq!(suspended.status, WorkflowStatus::Suspended);
    let terminal = suspended.terminal_reason.unwrap();
    assert!(
        terminal.contains("workflow agent returned invalid JSON"),
        "{terminal}"
    );
    assert!(terminal.contains("nope three"), "{terminal}");
    assert_eq!(mock.call_count(), 3, "1 initial ask + 2 corrections");
    let events = kinds(&store, "run-junk").await;
    assert_eq!(events.last().map(String::as_str), Some("runtime_error"));
}
