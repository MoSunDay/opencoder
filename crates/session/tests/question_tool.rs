//! End-to-end contract for the `question` tool: the model asks, an attached
//! listener resolves via the shared hub, and the answer lands as the tool
//! result in the SAME turn — feeding the follow-up LLM call. Also pins the
//! headless fallback (no listener → immediate fixed reply, never a hang) and
//! the cancel path (no dangling hub registration).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use opencoder_core::{resolve_agent, Config};
use opencoder_llm::tool_call::CompletedToolCall;
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient, Usage};
use opencoder_session::tools::question::QuestionHub;
use opencoder_session::{run, SessionEvent, SessionState};

fn config() -> Config {
    Config {
        model: "m/g".into(),
        ..Config::default()
    }
}

fn question_turn(id: &str, question: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: String::new(),
        tool_calls: vec![CompletedToolCall {
            id: id.into(),
            name: "question".into(),
            input: serde_json::json!({ "question": question, "options": ["sqlite", "postgres"] }),
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
            output_tokens: 1,
            total_tokens: 6,
            ..Default::default()
        }),
    }
}

fn sandbox_session(mock: Arc<MockChatClient>, hub: Option<Arc<QuestionHub>>) -> SessionState {
    let dir = tempfile::tempdir().unwrap();
    let mut s = SessionState::new(
        "question-1",
        resolve_agent("sandbox").unwrap(),
        config(),
        mock as Arc<dyn ChatStream>,
        dir.path().to_path_buf(),
    );
    if let Some(h) = hub {
        s = s.with_question_hub(h);
    }
    s
}

/// Resolve `id` on `hub` as soon as the tool registers (bounded polling —
/// `ToolStart` is emitted before execution, so an immediate resolve may race
/// ahead of `ask`).
async fn resolve_when_asked(hub: Arc<QuestionHub>, id: &str, answer: &str) {
    for _ in 0..1_000 {
        if hub.resolve(id, answer.to_string()) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("question tool never registered on the hub");
}

fn tool_end_for(events: &[SessionEvent], id: &str) -> Option<(String, bool)> {
    events.iter().find_map(|ev| match ev {
        SessionEvent::ToolEnd {
            id: eid,
            output,
            is_error,
            ..
        } if eid == id => Some((output.clone(), *is_error)),
        _ => None,
    })
}

/// Happy path: the answer becomes the Tool message content and is present in
/// the follow-up LLM request context (same turn, no extra user message).
#[tokio::test]
async fn answered_question_feeds_the_followup_call() {
    let hub = QuestionHub::new();
    hub.attach();
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![question_turn("q-1", "which database?")])
            .push_script(vec![text_done("## Plan\nuse postgres")]),
    );
    let mut session = sandbox_session(mock.clone(), Some(hub.clone()));
    let events: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();

    let resolver = tokio::spawn(resolve_when_asked(hub, "q-1", "postgres please"));
    let observed = sink;
    run(&mut session, "plan a migration".into(), move |ev| {
        observed.lock().unwrap().push(ev);
    })
    .await
    .unwrap();
    resolver.await.unwrap();

    let evs = events.lock().unwrap();
    let (output, is_error) = tool_end_for(&evs, "q-1").expect("ToolEnd for the question call");
    assert!(!is_error);
    assert_eq!(output, "postgres please");

    // The second LLM call must carry the answer as the tool result context.
    let reqs = mock.requests();
    assert_eq!(reqs.len(), 2, "question turn + follow-up turn");
    let second = serde_json::to_string(&reqs[1].to_body()).unwrap();
    assert!(
        second.contains("postgres please"),
        "answer in round-2 context"
    );
}

/// Esc / skip: the tool result is the skip text, turn completes normally.
#[tokio::test]
async fn skipped_question_returns_the_skip_text() {
    let hub = QuestionHub::new();
    hub.attach();
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![question_turn("q-2", "which port?")])
            .push_script(vec![text_done("## Plan\ndefault port")]),
    );
    let mut session = sandbox_session(mock, Some(hub.clone()));
    let events: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let observed = events.clone();

    let resolver = tokio::spawn(resolve_when_asked(
        hub,
        "q-2",
        "User skipped the question. Proceed with your best judgment.",
    ));
    run(&mut session, "plan it".into(), move |ev| {
        observed.lock().unwrap().push(ev);
    })
    .await
    .unwrap();
    resolver.await.unwrap();

    let evs = events.lock().unwrap();
    let (output, is_error) = tool_end_for(&evs, "q-2").expect("ToolEnd present");
    assert!(!is_error);
    assert!(
        output.contains("User skipped"),
        "skip text is the tool result"
    );
}

/// Headless (run/web): no listener attached → fixed fallback reply at once,
/// the turn never blocks on a human that is not there.
#[tokio::test]
async fn unattached_hub_falls_back_without_waiting() {
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![question_turn("q-3", "which format?")])
            .push_script(vec![text_done("## Plan\njson")]),
    );
    let mut session = sandbox_session(mock, None); // hub exists, never attached
    let events: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let observed = events.clone();

    // No resolver at all: completion within the bound proves no hang.
    tokio::time::timeout(
        Duration::from_secs(10),
        run(&mut session, "plan".into(), move |ev| {
            observed.lock().unwrap().push(ev);
        }),
    )
    .await
    .expect("turn completes without a listener")
    .unwrap();

    let evs = events.lock().unwrap();
    let (output, is_error) = tool_end_for(&evs, "q-3").expect("ToolEnd present");
    assert!(!is_error);
    assert!(
        output.contains("No interactive user"),
        "fallback reply, got: {output}"
    );
}

/// Turn interrupt while the question is pending: the tool returns an
/// interrupted error (no dangling hub registration), nothing hangs.
#[tokio::test]
async fn turn_cancel_unblocks_a_pending_question() {
    use opencoder_session::SharedCancel;
    use tokio_util::sync::CancellationToken;

    let hub = QuestionHub::new();
    hub.attach();
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![question_turn("q-4", "which language?")])
            .push_script(vec![text_done("## Plan")]),
    );
    let turn_cancel: SharedCancel = Arc::new(Mutex::new(CancellationToken::new()));
    let mut session = sandbox_session(mock, Some(hub.clone()));
    session = session.with_turn_cancel(turn_cancel.clone());

    let events: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let observed = events.clone();
    let fire = turn_cancel.clone();
    let res = tokio::time::timeout(
        Duration::from_secs(10),
        run(&mut session, "plan".into(), move |ev| {
            if matches!(&ev, SessionEvent::ToolStart { name, .. } if name == "question") {
                fire.lock().unwrap().cancel();
            }
            observed.lock().unwrap().push(ev);
        }),
    )
    .await
    .expect("cancel unblocks the pending question");

    let evs = events.lock().unwrap();
    let (_, is_error) = tool_end_for(&evs, "q-4").expect("ToolEnd present after cancel");
    assert!(is_error, "interrupted tool result is an error");
    assert_eq!(hub.waiting_count(), 0, "no dangling hub registration");
    // Bounded completion is the contract; the exact turn outcome may be Ok or Err.
    let _ = res;
}
