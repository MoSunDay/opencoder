//! Integration tests: hard-abort a MIXED tool batch (`task` + non-task) and
//! prove the transcript stays well-formed across an in-process continuation.
//!
//! Root cause this suite guards: the hard-cancel branch in `run_loop` used to
//! drop the entire tool message, leaving every `tool_use` id of the batch
//! unanswered. `resume()`'s dangling-tool_use reconciliation only runs on a
//! NEW process resume (`session resume` / `--continue`); an in-process
//! continuation (web drain, TUI double-Esc then continue, CLI retry) went
//! straight to `replay_cancelled_tasks`, which only backfills `task` ids — so
//! a non-task `tool_use` stayed dangling and the next LLM request hit the
//! provider's HTTP 400 ("unanswered tool_call").
//!
//! Fixes under test:
//! - `runner/mod.rs` hard-cancel branch: non-replayable results are recorded
//!   as a Tool message even under hard cancel; only replayable `task` ids stay
//!   dangling.
//! - `dangling_tools::reconcile_dangling_tool_uses` (called from
//!   `run_with_registry`): the in-process safety net answering any leftover
//!   dangling, non-replayable id before the next LLM request.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use opencoder_core::{resolve_agent, Config, ContentBlock};
use opencoder_llm::{ChatRequest, ChatStream, CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::{run, SessionEvent, SessionState};
use opencoder_store::{LibsqlStore, Store, SubagentStatus};
use tokio_util::sync::CancellationToken;

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

fn config() -> Config {
    Config {
        model: "m/g".into(),
        // Short grace window so a wedged subagent is force-cancelled quickly.
        subagent_drain_secs: Some(2),
        ..Config::default()
    }
}

fn mixed_batch_turn(task_prompt: &str, bash_cmd: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: "delegating + running bash".into(),
        tool_calls: vec![
            CompletedToolCall {
                id: "task-1".into(),
                name: "task".into(),
                input: serde_json::json!({"prompt": task_prompt, "subagent_type": "explore"}),
            },
            CompletedToolCall {
                id: "call_2".into(),
                name: "bash".into(),
                input: serde_json::json!({"command": bash_cmd}),
            },
        ],
        usage: Some(Usage {
            input_tokens: 10,
            output_tokens: 5,
            total_tokens: 15,
            ..Default::default()
        }),
    }
}

fn bash_call(cmd: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: String::new(),
        tool_calls: vec![CompletedToolCall {
            id: "child-bash".into(),
            name: "bash".into(),
            input: serde_json::json!({"command": cmd}),
        }],
        usage: Some(Usage {
            input_tokens: 5,
            output_tokens: 5,
            total_tokens: 10,
            ..Default::default()
        }),
    }
}

fn text_done(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![],
        usage: Some(Usage {
            input_tokens: 5,
            output_tokens: 5,
            total_tokens: 10,
            ..Default::default()
        }),
    }
}

/// Every assistant `tool_calls[].id` in every observed request must be
/// answered by a later `role: "tool"` message — the exact condition the
/// provider enforces (violation => HTTP 400).
fn assert_requests_well_formed(client: &MockChatClient) {
    let requests = client.requests();
    assert!(
        !requests.is_empty(),
        "expected at least one request against the mock"
    );
    for (i, req) in requests.iter().enumerate() {
        assert_no_dangling_in_request(i, req);
    }
}

fn assert_no_dangling_in_request(i: usize, req: &ChatRequest) {
    let mut pending: Vec<String> = Vec::new();
    for m in &req.messages {
        match m.get("role").and_then(|v| v.as_str()) {
            Some("assistant") => {
                if let Some(calls) = m.get("tool_calls").and_then(|v| v.as_array()) {
                    for call in calls {
                        if let Some(id) = call.get("id").and_then(|v| v.as_str()) {
                            pending.push(id.to_string());
                        }
                    }
                }
            }
            Some("tool") => {
                if let Some(id) = m.get("tool_call_id").and_then(|v| v.as_str()) {
                    pending.retain(|p| p != id);
                }
            }
            _ => {}
        }
    }
    assert!(
        pending.is_empty(),
        "request {i} carries unanswered tool_calls: {pending:?} (HTTP 400 condition)"
    );
}

/// Ids of `tool_use` blocks in the session transcript that no `tool_result`
/// answers.
fn dangling_tool_use_ids(s: &SessionState) -> Vec<String> {
    let answered: HashSet<&str> = s
        .messages
        .iter()
        .flat_map(|m| m.blocks.iter())
        .filter_map(|b| match b {
            ContentBlock::ToolResult { tool_use_id, .. } => Some(tool_use_id.as_str()),
            _ => None,
        })
        .collect();
    s.messages
        .iter()
        .flat_map(|m| m.blocks.iter())
        .filter_map(|b| match b {
            ContentBlock::ToolUse { id, .. } if !answered.contains(id.as_str()) => Some(id.clone()),
            _ => None,
        })
        .collect()
}

/// Block until the runner emits `SubagentStart`, bounding the wait so a broken
/// dispatch fails fast instead of hanging the whole test.
async fn wait_for_subagent_start(events: &Arc<Mutex<Vec<SessionEvent>>>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, SessionEvent::SubagentStart { .. }))
        {
            break;
        }
        if Instant::now() > deadline {
            panic!("subagent never started within 5s");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn hard_cancel_mixed_batch_records_non_task_result_then_continue_is_wellformed() {
    let store = mem_store().await;
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![mixed_batch_turn("explore something", "sleep 30")])
            .push_script(vec![bash_call("sleep 30")])
            .push_script(vec![text_done("recovered")]),
    );
    let mock_ref = mock.clone();
    let mock: Arc<dyn ChatStream> = mock;

    let agent = resolve_agent("act").unwrap();
    let cancel = CancellationToken::new();
    let mut session =
        SessionState::new("mixed-abort", agent, config(), mock, std::env::temp_dir())
            .with_cancel(cancel.clone())
            .with_store(store.clone());
    let session_id = session.id.clone();

    // Run 1: dispatch [task, bash sleep 30], cancel hard once the child is
    // mid-bash. A separate task owns the cancel so run 1 returns promptly.
    let events: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let events_for_wait = events.clone();
    let cancel_for_wait = cancel.clone();
    let waiter = tokio::spawn(async move {
        wait_for_subagent_start(&events_for_wait).await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        cancel_for_wait.cancel();
    });
    let sink = events.clone();
    let r1 = tokio::time::timeout(
        Duration::from_secs(15),
        run(&mut session, "go".into(), move |ev| {
            sink.lock().unwrap().push(ev);
        }),
    )
    .await;
    let _ = waiter.await;
    assert!(
        r1.is_ok(),
        "run 1 did not complete within 15s; hard-abort during mixed batch is broken"
    );

    // Fix A: the non-task result was recorded even under hard cancel, so the
    // transcript holds a tool message answering call_2; task-1 stays dangling
    // (its subagent is Cancelled and gets replayed/abandoned on the next turn).
    {
        let answered: HashSet<&str> = session
            .messages
            .iter()
            .flat_map(|m| m.blocks.iter())
            .filter_map(|b| match b {
                ContentBlock::ToolResult { tool_use_id, .. } => Some(tool_use_id.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            answered.contains("call_2"),
            "run 1 must record the interrupted bash result for call_2, got answered={answered:?}"
        );
        let dangling = dangling_tool_use_ids(&session);
        assert_eq!(
            dangling,
            vec!["task-1".to_string()],
            "only the replayable task id may stay dangling after run 1"
        );
    }
    let tasks = store.list_subagent_tasks(&session_id).await.unwrap();
    assert_eq!(tasks.len(), 1, "expected exactly one subagent task");
    assert!(
        matches!(tasks[0].status, SubagentStatus::Cancelled),
        "task must be Cancelled after hard abort, got {:?}",
        tasks[0].status
    );

    // Run 2: fresh token, continue in-process. The abandoned task is
    // backfilled (has_new_input) and the transcript must be well-formed.
    session = session.with_cancel(CancellationToken::new());
    let events2: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let sink2 = events2.clone();
    let r2 = tokio::time::timeout(
        Duration::from_secs(15),
        run(&mut session, "again".into(), move |ev| {
            sink2.lock().unwrap().push(ev);
        }),
    )
    .await;
    assert!(
        r2.is_ok(),
        "run 2 did not complete within 15s; continuing after abort is broken"
    );
    let saw_done = events2
        .lock()
        .unwrap()
        .iter()
        .any(|e| matches!(e, SessionEvent::Done));
    assert!(saw_done, "expected Done event after continuing post-abort");

    let dangling = dangling_tool_use_ids(&session);
    assert!(
        dangling.is_empty(),
        "transcript must be well-formed after continue, dangling={dangling:?}"
    );
    // The exact HTTP-400 condition: every observed request has every
    // assistant tool_call answered by a matching tool message.
    assert_requests_well_formed(&mock_ref);
}

#[tokio::test]
async fn in_process_continue_reconciles_preexisting_dangling_non_task() {
    let store = mem_store().await;
    let mock = Arc::new(MockChatClient::new().push_script(vec![text_done("done")]));
    let mock_ref = mock.clone();
    let mock: Arc<dyn ChatStream> = mock;

    let agent = resolve_agent("act").unwrap();
    let mut session =
        SessionState::new("dangling-reconcile", agent, config(), mock, std::env::temp_dir())
            .with_store(store.clone());

    // Hand-build a transcript with a dangling non-task tool_use — the exact
    // state a mid-batch hard cancel used to leave behind (tool message
    // dropped). Not a task id, so replay_cancelled_tasks cannot fix it.
    session
        .record(opencoder_core::Message::user(
            opencoder_session::runner::new_id(),
            "build it",
        ))
        .await;
    session
        .record(opencoder_core::Message {
            id: opencoder_session::runner::new_id(),
            role: opencoder_core::Role::Assistant,
            blocks: vec![ContentBlock::ToolUse {
                id: "call_orphan".into(),
                name: "bash".into(),
                input: serde_json::json!({"command": "echo hi"}),
            }],
            model: None,
            agent: None,
            usage: opencoder_core::MessageUsage::default(),
            created_at: opencoder_core::message::now_ms(),
            synthetic: false,
        })
        .await;

    // In-process continuation: the safety net must answer the orphan BEFORE
    // the new user input enters the loop.
    let events: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    run(&mut session, "again".into(), move |ev| {
        sink.lock().unwrap().push(ev);
    })
    .await
    .unwrap();

    let dangling = dangling_tool_use_ids(&session);
    assert!(
        dangling.is_empty(),
        "in-process continue must reconcile the dangling orphan, dangling={dangling:?}"
    );
    // The synthesized tool message lands at the end of the pre-existing
    // transcript (index 2: user, assistant(tool_use), tool(error)), i.e.
    // right before the new user turn.
    let synth = &session.messages[2];
    assert!(synth.synthetic, "synthesized tool message must be flagged synthetic");
    assert_eq!(synth.role, opencoder_core::Role::Tool);
    let orphan_result = synth.blocks.iter().any(|b| {
        matches!(
            b,
            ContentBlock::ToolResult {
                tool_use_id,
                is_error: true,
                ..
            } if tool_use_id == "call_orphan"
        )
    });
    assert!(orphan_result, "expected synthetic error ToolResult for call_orphan");

    // And the mock must never have been asked to produce a request carrying an
    // unanswered tool_call (the transcript the provider would reject).
    assert_requests_well_formed(&mock_ref);
}
