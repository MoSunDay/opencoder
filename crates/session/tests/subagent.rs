//! Subagent dispatch tests — verifies run_subagent emits SubagentStart/SubagentEnd
//! and forwards child events to the parent's on_event sink.
//!
//! Contracts:
//! - task tool call triggers SubagentStart + child run + SubagentEnd(ok=true)
//! - empty prompt returns error without SubagentStart
//! - with a store attached, parent-child relationship + completion are persisted
//!   to the subagent_tasks table and child events land in session_events.

use std::sync::Arc;

use opencode_core::{resolve_agent, Config};
use opencode_llm::{CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencode_session::{run, SessionEvent, SessionState};
use opencode_store::{LibsqlStore, Store, SubagentStatus};

fn config() -> Config {
    Config {
        model: "m/g".into(),
        ..Config::default()
    }
}

fn task_turn(prompt: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: "delegating".into(),
        tool_calls: vec![CompletedToolCall {
            id: "task-1".into(),
            name: "task".into(),
            input: serde_json::json!({"prompt": prompt, "subagent_type": "explore"}),
        }],
        usage: Some(Usage {
            input_tokens: 10,
            output_tokens: 5,
            total_tokens: 15,
        }),
    }
}

/// Parent turn emitting TWO `task` calls in a single response — the runner
/// dispatches them concurrently (FuturesUnordered) rather than serially.
fn two_task_turn() -> LlmEvent {
    LlmEvent::Completed {
        text: "delegating to two".into(),
        tool_calls: vec![
            CompletedToolCall {
                id: "task-A".into(),
                name: "task".into(),
                input: serde_json::json!({"prompt": "job A", "subagent_type": "explore"}),
            },
            CompletedToolCall {
                id: "task-B".into(),
                name: "task".into(),
                input: serde_json::json!({"prompt": "job B", "subagent_type": "explore"}),
            },
        ],
        usage: Some(Usage {
            input_tokens: 10,
            output_tokens: 5,
            total_tokens: 15,
        }),
    }
}

fn text_done(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![],
        usage: None,
    }
}

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

#[tokio::test]
async fn subagent_emits_start_and_end_events() {
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![task_turn("research the codebase")]) // parent turn 1: task call
            .push_script(vec![text_done("found 3 files")]) // child turn 1: text done
            .push_script(vec![text_done("all done")]), // parent turn 2: done
    );

    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let mut session =
        SessionState::new("sub-test", agent, config(), mock, dir.path().to_path_buf());

    let mut events = Vec::new();
    run(&mut session, "delegate research".into(), |ev| {
        events.push(ev)
    })
    .await
    .unwrap();

    let has_start = events.iter().any(|e| {
        matches!(
            e,
            SessionEvent::SubagentStart { kind, prompt, .. }
            if kind == "explore" && prompt.contains("research")
        )
    });
    assert!(
        has_start,
        "expected SubagentStart, got {:?}",
        events.iter().map(format_ev).collect::<Vec<_>>()
    );

    let has_end = events.iter().any(|e| {
        matches!(
            e,
            SessionEvent::SubagentEnd { ok: true, summary, .. }
            if summary.contains("found")
        )
    });
    assert!(
        has_end,
        "expected SubagentEnd(ok=true, summary contains 'found'), got {:?}",
        events.iter().map(format_ev).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn concurrent_subagent_dispatch_in_one_turn() {
    // Parent emits TWO task calls in one turn. The runner fans them out
    // concurrently (FuturesUnordered); both children run to completion and the
    // parent collects both results. Each child emits SubagentStart (running)
    // and SubagentEnd (finished), so the user sees both lifecycle signals.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![two_task_turn()]) // parent turn 1: two task calls
            .push_script(vec![text_done("result A")]) // child 1 turn
            .push_script(vec![text_done("result B")]) // child 2 turn
            .push_script(vec![text_done("parent done")]), // parent turn 2: done
    );

    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let mut session = SessionState::new(
        "sub-concurrent",
        agent,
        config(),
        mock,
        dir.path().to_path_buf(),
    );

    let mut events = Vec::new();
    run(&mut session, "delegate two jobs".into(), |ev| {
        events.push(ev)
    })
    .await
    .unwrap();

    let starts = events
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(e, SessionEvent::SubagentStart { .. }))
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    assert_eq!(
        starts.len(),
        2,
        "expected 2 SubagentStart (running) events, got {:?}",
        events.iter().map(format_ev).collect::<Vec<_>>()
    );

    let first_end_idx = events
        .iter()
        .position(|e| matches!(e, SessionEvent::SubagentEnd { .. }))
        .unwrap_or(usize::MAX);
    assert!(
        starts[1] < first_end_idx,
        "second SubagentStart must precede first SubagentEnd (concurrent overlap): starts={starts:?} first_end={first_end_idx}"
    );

    let ends: Vec<(bool, String)> = events
        .iter()
        .filter_map(|e| match e {
            SessionEvent::SubagentEnd { ok, summary, .. } => Some((*ok, summary.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(ends.len(), 2, "expected 2 SubagentEnd (finished) events");
    assert!(
        ends.iter().all(|(ok, _)| *ok),
        "both subagents should succeed: {:?}",
        ends
    );

    let joined = ends
        .iter()
        .map(|(_, s)| s.as_str())
        .collect::<Vec<_>>()
        .join(" || ");
    assert!(
        joined.contains("result A") && joined.contains("result B"),
        "both child results should be forwarded to the parent: {joined}"
    );
}

#[tokio::test]
async fn subagent_wraps_child_events_in_subagent_child() {
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![task_turn("run a command")]) // parent: task call
            .push_script(vec![LlmEvent::Completed {
                // child: bash call
                text: "".into(),
                tool_calls: vec![CompletedToolCall {
                    id: "child-bash".into(),
                    name: "bash".into(),
                    input: serde_json::json!({"command": "echo hi"}),
                }],
                usage: Some(Usage {
                    input_tokens: 5,
                    output_tokens: 1,
                    total_tokens: 6,
                }),
            }])
            .push_script(vec![text_done("done")]) // child: done
            .push_script(vec![text_done("parent done")]), // parent: done
    );

    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let mut session = SessionState::new("sub-fwd", agent, config(), mock, dir.path().to_path_buf());

    let mut events = Vec::new();
    run(&mut session, "delegate".into(), |ev| events.push(ev))
        .await
        .unwrap();

    // Child's bash ToolStart should arrive wrapped in SubagentChild
    let has_child_tool = events.iter().any(|e| {
        matches!(
            e,
            SessionEvent::SubagentChild { ev, .. }
                if matches!(ev.as_ref(), SessionEvent::ToolStart { name, .. } if name == "bash")
        )
    });
    assert!(
        has_child_tool,
        "expected child bash ToolStart wrapped in SubagentChild"
    );

    let has_subagent_end = events
        .iter()
        .any(|e| matches!(e, SessionEvent::SubagentEnd { .. }));
    assert!(has_subagent_end, "expected SubagentEnd");
}

#[tokio::test]
async fn subagent_persists_parent_child_to_store() {
    let store = mem_store().await;
    store
        .create_session(&opencode_store::SessionMeta {
            id: "sub-persist".into(),
            title: Some("t".into()),
            agent: Some("act".into()),
            model: Some("m".into()),
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
        })
        .await
        .unwrap();

    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![task_turn("explore the codebase structure")])
            .push_script(vec![text_done("found main.rs and lib.rs")])
            .push_script(vec![text_done("parent done")]),
    );

    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let mut session = SessionState::new(
        "sub-persist",
        agent,
        config(),
        mock,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone());

    let mut events = Vec::new();
    run(&mut session, "delegate".into(), |ev| events.push(ev))
        .await
        .unwrap();

    // The parent-child relationship must be in subagent_tasks with Completed status.
    let tasks = store.list_subagent_tasks("sub-persist").await.unwrap();
    assert_eq!(
        tasks.len(),
        1,
        "expected 1 subagent task row, got {}",
        tasks.len()
    );
    let t = &tasks[0];
    assert_eq!(t.agent, "explore");
    assert!(t.prompt.contains("explore the codebase"));
    assert!(
        matches!(t.status, SubagentStatus::Completed),
        "status must be Completed"
    );
    assert_eq!(t.ok, Some(true), "ok must be true");
    assert!(
        t.result.as_deref().unwrap_or("").contains("found main.rs"),
        "result must contain child output, got: {:?}",
        t.result
    );
    assert!(
        t.child_session_id.starts_with("sub-"),
        "child session id must be sub-prefixed"
    );
    assert!(t.completed_at.is_some(), "completed_at must be set");

    // The child session row must exist with the correct metadata from the
    // explicit seed (not from persist()'s auto-create). This guards against
    // a regression where double-create_session overwrites the runner's metadata.
    let child_meta = store.get_session(&t.child_session_id).await.unwrap();
    assert!(child_meta.is_some(), "child session row must exist");
    let cm = child_meta.unwrap();
    assert_eq!(
        cm.agent.as_deref(),
        Some("explore"),
        "child agent must be the subagent kind"
    );
    assert!(
        cm.title
            .as_deref()
            .unwrap_or("")
            .contains("explore the codebase"),
        "child title must be the truncated prompt, got: {:?}",
        cm.title
    );
}

#[tokio::test]
async fn subagent_persists_child_events_to_store() {
    let store = mem_store().await;
    store
        .create_session(&opencode_store::SessionMeta {
            id: "sub-ev".into(),
            title: Some("t".into()),
            agent: Some("act".into()),
            model: Some("m".into()),
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
        })
        .await
        .unwrap();

    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![task_turn("do work")])
            .push_script(vec![LlmEvent::Completed {
                text: "child working".into(),
                tool_calls: vec![CompletedToolCall {
                    id: "child-tool".into(),
                    name: "bash".into(),
                    input: serde_json::json!({"command": "echo hello"}),
                }],
                usage: Some(Usage {
                    input_tokens: 5,
                    output_tokens: 1,
                    total_tokens: 6,
                }),
            }])
            .push_script(vec![text_done("child finished")])
            .push_script(vec![text_done("parent done")]),
    );

    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let mut session = SessionState::new("sub-ev", agent, config(), mock, dir.path().to_path_buf())
        .with_store(store.clone());

    run(&mut session, "delegate".into(), |_| {}).await.unwrap();

    // Find the child session id from the task record.
    let tasks = store.list_subagent_tasks("sub-ev").await.unwrap();
    assert_eq!(tasks.len(), 1);
    let child_id = &tasks[0].child_session_id;

    // Child events must be persisted to session_events (fire-and-forget, so
    // give the detached spawns a moment to flush).
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let events = store.events_after(child_id, 0).await.unwrap();
    assert!(
        !events.is_empty(),
        "expected child events persisted for {child_id}"
    );
}

#[tokio::test]
async fn subagent_rejects_unknown_type() {
    // A bogus subagent_type must produce a descriptive error, not silently
    // fall back to explore.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![LlmEvent::Completed {
                text: "".into(),
                tool_calls: vec![CompletedToolCall {
                    id: "task-bad".into(),
                    name: "task".into(),
                    input: serde_json::json!({"prompt": "do stuff", "subagent_type": "ninja"}),
                }],
                usage: Some(Usage {
                    input_tokens: 5,
                    output_tokens: 1,
                    total_tokens: 6,
                }),
            }])
            .push_script(vec![text_done("ok")]),
    );
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let mut session = SessionState::new("sub-bad", agent, config(), mock, dir.path().to_path_buf());

    let mut events = Vec::new();
    run(&mut session, "delegate".into(), |ev| events.push(ev))
        .await
        .unwrap();

    // The task tool call must produce a ToolEnd with is_error=true mentioning
    // the unknown type.
    let tool_end = events.iter().find(|e| {
        matches!(
            e,
            SessionEvent::ToolEnd { name, .. } if name == "task"
        )
    });
    assert!(tool_end.is_some(), "expected a ToolEnd for the task tool");
    if let SessionEvent::ToolEnd {
        is_error, output, ..
    } = tool_end.unwrap()
    {
        assert!(*is_error, "unknown subagent_type must error");
        assert!(
            output.contains("Unknown subagent_type") && output.contains("ninja"),
            "error must name the bad type, got: {output}"
        );
    }

    // No SubagentStart should have been emitted.
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, SessionEvent::SubagentStart { .. })),
        "must not start a subagent for an unknown type"
    );
}

fn format_ev(e: &SessionEvent) -> &'static str {
    match e {
        SessionEvent::TextDelta(_) => "TextDelta",
        SessionEvent::ToolStart { .. } => "ToolStart",
        SessionEvent::ToolEnd { .. } => "ToolEnd",
        SessionEvent::SubagentStart { .. } => "SubagentStart",
        SessionEvent::SubagentEnd { .. } => "SubagentEnd",
        SessionEvent::SubagentChild { .. } => "SubagentChild",
        SessionEvent::Done => "Done",
        SessionEvent::Error(_) => "Error",
        _ => "Other",
    }
}
