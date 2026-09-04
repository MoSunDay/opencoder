//! Integration regressions for the todos bug sweep that need the full
//! runtime (real store + mock model): recovery_context reaching retry
//! prompts (M3), the parent session's compaction metadata staying clean
//! (M4), correctable non-dispatch decisions (M6), idempotent milestone
//! re-marks (M6) and resume persisting Running before the first dispatch
//! (L1).

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use opencoder_core::{Config, Role};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
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

const CANDIDATE: &str = r#"{"status":"candidate","summary":"done","result":"ok","verification":"checked","evidence_refs":[],"recovery_context":{"summary":"done","refs":[]}}"#;

const BLOCKED: &str = r#"{"status":"blocked","summary":"blocked-after-first-probe","result":null,"verification":"blocked","evidence_refs":[],"recovery_context":{"summary":"blocked-after-first-probe","refs":["note-1"]}}"#;

fn spec(gate: bool) -> WorkflowSpec {
    let mut out = spec_with_required_bash("cat done.txt");
    if !gate {
        out.todos[0].acceptance.required_tool_calls = Vec::new();
    }
    out
}

/// Same single-todo workflow, but the acceptance contract demands a real
/// successful `bash` run whose command contains `needle` (the B1 positive
/// test uses an always-succeeding command; the M6 negative keeps `spec(true)`).
fn spec_with_required_bash(needle: &str) -> WorkflowSpec {
    let todo = TodoSpec {
        id: "step-1".into(),
        title: "step".into(),
        requirement_background: "required by test".into(),
        instructions: "return the candidate".into(),
        depends_on: Vec::new(),
        agent: "act".into(),
        max_attempts: 3,
        acceptance: AcceptanceSpec {
            criteria: "candidate exists".into(),
            required_tool_calls: required_bash(needle),
        },
        metadata: serde_json::Value::Null,
    };
    WorkflowSpec {
        schema_version: 1,
        id: "wf-guards".into(),
        name: "guards".into(),
        objective: "finish one item".into(),
        constraints: Vec::new(),
        todos: vec![todo],
        metadata: serde_json::Value::Null,
    }
}

/// A `bash` tool-call turn whose command contains `needle` (for acceptance
/// gates that demand a real tool run).
fn required_bash(needle: &str) -> Vec<RequiredToolCall> {
    vec![RequiredToolCall {
        name: "bash".into(),
        arguments_contains: serde_json::json!({"command": needle}),
        result_ok: true,
    }]
}

/// One LLM round that really invokes the `bash` tool with an
/// always-succeeding command (the follow-up round carries the candidate).
fn bash_tool_turn(command: &str) -> Vec<LlmEvent> {
    vec![LlmEvent::Completed {
        text: "running the gate tool".into(),
        tool_calls: vec![opencoder_llm::CompletedToolCall {
            id: "t1".into(),
            name: "bash".into(),
            input: serde_json::json!({"command": command}),
        }],
        usage: None,
    }]
}

fn runtime(store: &Arc<dyn Store>, client: Arc<dyn ChatStream>, dir: &std::path::Path) -> Runtime {
    Runtime {
        store: store.clone(),
        client,
        config: Config::default(),
        workdir: dir.to_path_buf(),
        debug_root: None,
        cancel: CancellationToken::new(),
    }
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

/// M3: the previous attempt's blocked candidate carries recovery_context into
/// the retry prompt (PREVIOUS_RECOVERY). dispatch used to clear the candidate
/// before the execution snapshot was taken, so the channel was always null.
#[tokio::test]
async fn recovery_context_reaches_retry_prompt() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(dispatch("step-1", "new"))
            .push_script(done(BLOCKED))
            .push_script(dispatch("step-1", "resume"))
            .push_script(done(CANDIDATE))
            .push_script(done(
                r#"{"operation":"accept","reason":"ok","mark_milestone":false}"#,
            ))
            .push_script(done(r#"{"operation":"complete","reason":"all passed"}"#)),
    );
    let temp = tempfile::tempdir().unwrap();
    let state = runtime(&store, mock, temp.path())
        .run_new_with_id(spec(false), "run-recovery".into())
        .await
        .unwrap();
    assert_eq!(state.status, WorkflowStatus::Completed);

    let session_id = state.todos["step-1"].active_session_id.clone().unwrap();
    let prompts: Vec<String> = store
        .load_messages(&session_id)
        .await
        .unwrap()
        .iter()
        .filter(|message| message.role == Role::User)
        .map(|message| {
            message
                .blocks
                .iter()
                .filter_map(|block| block.as_text())
                .collect::<String>()
        })
        .filter(|text| text.contains("Complete exactly one focused TODO"))
        .collect();
    assert_eq!(prompts.len(), 2, "both attempts' focused prompts persist");
    assert!(
        prompts[0].contains("PREVIOUS_RECOVERY=null"),
        "first attempt has no previous candidate: {}",
        prompts[0]
    );
    assert!(
        prompts[1].contains("PREVIOUS_RECOVERY=")
            && prompts[1].contains("blocked-after-first-probe")
            && prompts[1].contains("note-1"),
        "retry prompt must carry the blocked candidate's recovery context: {}",
        prompts[1]
    );
}

/// M4: decide() must not touch the parent session's summary/summary_seq —
/// they are compaction metadata. The transcript must stay intact after
/// several decisions.
#[tokio::test]
async fn parent_summary_stays_clean_across_decisions() {
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
    let state = runtime(&store, mock, temp.path())
        .run_new_with_id(spec(false), "run-summary".into())
        .await
        .unwrap();
    assert_eq!(state.status, WorkflowStatus::Completed);

    let parent = store
        .get_session(&state.parent_session_id)
        .await
        .unwrap()
        .expect("parent session exists");
    assert_eq!(parent.summary, None, "compaction summary must not be faked");
    assert_eq!(
        parent.summary_seq, None,
        "compaction watermark must stay unset"
    );
    let transcript = store.load_messages(&state.parent_session_id).await.unwrap();
    assert!(
        transcript.len() >= 4,
        "the full decision transcript must survive (got {} messages)",
        transcript.len()
    );
}

/// M6: a non-dispatch decision (rewind to a non-milestone) gets the same
/// correct-and-re-ask treatment as dispatch — one mistake must not suspend
/// the workflow.
#[tokio::test]
async fn nondispatch_decision_correction_reasks() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mock = Arc::new(
        MockChatClient::new()
            // Invalid: "step-1" is not (yet) a milestone and not even passed.
            .push_script(done(
                r#"{"operation":"rewind","milestone_todo_id":"step-1","reason":"drift"}"#,
            ))
            .push_script(dispatch("step-1", "new"))
            .push_script(done(CANDIDATE))
            .push_script(done(
                r#"{"operation":"accept","reason":"ok","mark_milestone":true}"#,
            ))
            .push_script(done(r#"{"operation":"complete","reason":"all passed"}"#)),
    );
    let temp = tempfile::tempdir().unwrap();
    let state = runtime(&store, mock.clone(), temp.path())
        .run_new_with_id(spec(false), "run-nondispatch".into())
        .await
        .unwrap();
    assert_eq!(state.status, WorkflowStatus::Completed);
    assert!(state.milestones.contains("step-1"));
    assert_eq!(
        mock.call_count(),
        5,
        "the invalid rewind consumed one re-ask"
    );
    let events = kinds(&store, "run-nondispatch").await;
    assert!(!events.iter().any(|kind| kind == "runtime_error"));
    assert!(!events.iter().any(|kind| kind == "workflow_rewound"));
}

/// M6②: re-marking an existing milestone is an idempotent no-op (the
/// rewind-recovery flow legitimately re-marks after re-acceptance), not a
/// workflow-fatal bail.
#[tokio::test]
async fn duplicate_milestone_remark_is_idempotent() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(dispatch("step-1", "new"))
            .push_script(done(CANDIDATE))
            .push_script(done(
                r#"{"operation":"accept","reason":"ok","mark_milestone":true}"#,
            ))
            // Duplicate: the milestone set already contains step-1.
            .push_script(done(
                r#"{"operation":"mark_milestone","todo_id":"step-1","reason":"again"}"#,
            ))
            .push_script(done(r#"{"operation":"complete","reason":"all passed"}"#)),
    );
    let temp = tempfile::tempdir().unwrap();
    let state = runtime(&store, mock.clone(), temp.path())
        .run_new_with_id(spec(false), "run-remark".into())
        .await
        .unwrap();
    assert_eq!(state.status, WorkflowStatus::Completed);
    assert!(state.milestones.contains("step-1"));
    assert_eq!(mock.call_count(), 5);
    let events = kinds(&store, "run-remark").await;
    assert!(!events.iter().any(|kind| kind == "runtime_error"));
    assert_eq!(
        events
            .iter()
            .filter(|kind| *kind == "milestone_marked")
            .count(),
        0,
        "the idempotent re-mark commits nothing"
    );
}

/// M6: acceptance decisions get correct-and-re-ask too. Accepting a failed
/// tool gate is invalid; the parent corrects to revise, and only a decision
/// that stays invalid after the budget actually ends the workflow.
/// (Post-B1 this negative still means "no tool ever ran" — a real failure.
/// The old bug was orthogonal: tools DID run but their events never reached
/// the gate because the run callback dropped them; see
/// `required_gate_passes_when_tool_really_ran` for the positive counterpart.)
#[tokio::test]
async fn acceptance_correction_reasks_on_failed_gate() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(dispatch("step-1", "new"))
            // No tools ran -> gate.ok=false.
            .push_script(done(CANDIDATE))
            // Invalid: accept despite failed gate -> correction re-ask.
            .push_script(done(
                r#"{"operation":"accept","reason":"looks fine","mark_milestone":false}"#,
            ))
            .push_script(done(
                r#"{"operation":"revise","reason":"run the gate tool","context_mode":"resume"}"#,
            ))
            .push_script(dispatch("step-1", "resume"))
            .push_script(done(CANDIDATE))
            // Still gate-failed, but fail is a VALID decision -> applied.
            .push_script(done(
                r#"{"operation":"fail","reason":"gate never satisfied"}"#,
            ))
            // Scheduling continues on the Failed todo: fail the workflow.
            .push_script(done(r#"{"operation":"fail","reason":"todo failed"}"#)),
    );
    let temp = tempfile::tempdir().unwrap();
    let state = runtime(&store, mock.clone(), temp.path())
        .run_new_with_id(spec(true), "run-gate".into())
        .await
        .unwrap();
    assert_eq!(state.status, WorkflowStatus::Failed);
    assert_eq!(state.todos["step-1"].status, TodoStatus::Failed);
    assert_eq!(mock.call_count(), 8);
    let events = kinds(&store, "run-gate").await;
    assert!(events.contains(&"todo_revision_requested".into()));
    assert!(events.contains(&"workflow_failed".into()));
    assert!(!events.iter().any(|kind| kind == "runtime_error"));
    assert!(!events.iter().any(|kind| kind == "todo_accepted"));
}

/// B1 positive counterpart to the M6 gate test above: when the TODO session
/// really runs the required tool, the acceptance gate must see the run and
/// pass — the workflow completes with the todo Passed and `todo_accepted`.
/// This locks in BOTH halves of the fix: the events reach `evaluate_gate`
/// (in-memory) AND they are persisted to the store (web-drain parity), so
/// observers replaying the todo session see the ToolStart/ToolEnd stream.
#[tokio::test]
async fn required_gate_passes_when_tool_really_ran() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(dispatch("step-1", "new"))
            // The gate demands a real `bash` run; `echo hi` always succeeds.
            // One script per LLM round: tool round, then candidate round.
            .push_script(bash_tool_turn("echo hi"))
            .push_script(done(CANDIDATE))
            // The gate now passes, so accept is a VALID decision.
            .push_script(done(
                r#"{"operation":"accept","reason":"gate satisfied","mark_milestone":false}"#,
            ))
            .push_script(done(r#"{"operation":"complete","reason":"all passed"}"#)),
    );
    let temp = tempfile::tempdir().unwrap();
    let state = runtime(&store, mock.clone(), temp.path())
        .run_new_with_id(spec_with_required_bash("echo hi"), "run-gate-ok".into())
        .await
        .unwrap();
    // gate.ok=true: the todo passed and was accepted (no revision re-ask).
    assert_eq!(state.status, WorkflowStatus::Completed);
    assert_eq!(state.todos["step-1"].status, TodoStatus::Passed);
    assert_eq!(
        mock.call_count(),
        5,
        "no correction re-ask on a passed gate"
    );
    let events = kinds(&store, "run-gate-ok").await;
    assert!(events.contains(&"todo_accepted".into()));
    assert!(events.contains(&"workflow_completed".into()));
    assert!(!events.iter().any(|kind| kind == "todo_revision_requested"));

    // Persistence half of the fix: the todo session's events (including the
    // real tool run) must be in the store, not just in memory.
    let sid = state.todos["step-1"].active_session_id.clone().unwrap();
    let records = store.events_after(&sid, -1).await.unwrap();
    assert!(!records.is_empty(), "todo session events must be persisted");
    let kinds_persisted: Vec<&str> = records
        .iter()
        .filter_map(|record| record.sse_kind.as_deref())
        .collect();
    assert!(kinds_persisted.contains(&"tool_start"));
    assert!(kinds_persisted.contains(&"tool_end"));
}

/// L1: a resumed workflow must be persisted as Running at the resume
/// boundary (workflow_resumed), before any dispatch — observers must not see
/// a Suspended workflow while the parent is already deciding.
#[tokio::test]
async fn resume_persists_running_before_first_dispatch() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mock = Arc::new(
        MockChatClient::new().push_script(done(r#"{"operation":"suspend","reason":"park"}"#)),
    );
    let temp = tempfile::tempdir().unwrap();
    let first = runtime(&store, mock, temp.path());
    let parked = first
        .run_new_with_id(spec(false), "run-resume".into())
        .await
        .unwrap();
    assert_eq!(parked.status, WorkflowStatus::Suspended);

    // The resumed drive hangs inside the FIRST parent decision: whatever is
    // persisted at that point is exactly the resume boundary state.
    let hang = Arc::new(tokio::sync::Notify::new());
    let mock2 = Arc::new(MockChatClient::new().push_hang(hang.clone()));
    let resume_runtime = runtime(&store, mock2, temp.path());
    let spawned = tokio::spawn(async move { resume_runtime.resume("run-resume").await });

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let record = store
            .get_todo_workflow("run-resume")
            .await
            .unwrap()
            .unwrap();
        if record.status == "running" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "workflow never persisted running at the resume boundary"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let events = kinds(&store, "run-resume").await;
    assert!(events.contains(&"workflow_resumed".into()));
    assert!(
        !events.iter().any(|kind| kind == "todos_dispatched"),
        "Running must be visible before the first dispatch, not only at it"
    );

    // Release the hang (empty stream): the decision fails and the drive
    // suspends — fine, the boundary observation above is the contract.
    hang.notify_one();
    let _ = tokio::time::timeout(Duration::from_secs(10), spawned)
        .await
        .expect("resume task finished after hang release");
}

/// T-2: a rewind to an unrelated milestone must not strand the TODO under
/// acceptance in `Accepting` forever (runnable() never proposes Accepting,
/// and only this acceptance flow can exit it). The decision is rejected and
/// corrected instead; the workflow runs to completion.
#[tokio::test]
async fn acceptance_rewind_to_unrelated_milestone_is_corrected() {
    // Two independent todos: "root" will be the milestone, "step-1" the
    // unrelated accepting todo the rewind decision targets.
    let mut workflow = spec(false);
    workflow.todos.push(TodoSpec {
        id: "root".into(),
        title: "root".into(),
        requirement_background: "independent root".into(),
        instructions: "return the candidate".into(),
        depends_on: vec![],
        agent: "act".into(),
        max_attempts: 3,
        acceptance: AcceptanceSpec {
            criteria: "candidate exists".into(),
            required_tool_calls: vec![],
        },
        metadata: serde_json::Value::Null,
    });
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mock = Arc::new(
        MockChatClient::new()
            // root passes and is marked as the milestone.
            .push_script(dispatch("root", "new"))
            .push_script(done(CANDIDATE))
            .push_script(done(
                r#"{"operation":"accept","reason":"ok","mark_milestone":true}"#,
            ))
            // step-1 accepted... no wait: the rewind decision arrives during
            // step-1's acceptance, pointing at the unrelated "root" milestone.
            .push_script(dispatch("step-1", "new"))
            .push_script(done(CANDIDATE))
            // Invalid: root is a milestone but step-1 is not in its subtree.
            .push_script(done(
                r#"{"operation":"rewind","milestone_todo_id":"root","reason":"drift"}"#,
            ))
            .push_script(done(
                r#"{"operation":"accept","reason":"ok","mark_milestone":false}"#,
            ))
            .push_script(done(r#"{"operation":"complete","reason":"all passed"}"#)),
    );
    let temp = tempfile::tempdir().unwrap();
    let state = runtime(&store, mock.clone(), temp.path())
        .run_new_with_id(workflow, "run-t2".into())
        .await
        .unwrap();
    assert_eq!(state.status, WorkflowStatus::Completed);
    assert_eq!(state.todos["step-1"].status, TodoStatus::Passed);
    assert_eq!(
        mock.call_count(),
        8,
        "the unrelated rewind consumed exactly one correction re-ask"
    );
    let events = kinds(&store, "run-t2").await;
    assert!(
        !events.iter().any(|kind| kind == "workflow_rewound"),
        "no rewind to the unrelated milestone may be applied"
    );
    assert!(!events.iter().any(|kind| kind == "runtime_error"));
}
