//! Provider-round lifecycle used by the TUI `[turn cost]` display.

use std::sync::{Arc, Mutex};

use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::{run, SessionEvent, SessionState};

fn completed(text: &str, tool_calls: Vec<CompletedToolCall>) -> LlmEvent {
    LlmEvent::Completed {
        text: text.into(),
        tool_calls,
        usage: None,
    }
}

fn bash(id: &str, command: &str) -> CompletedToolCall {
    CompletedToolCall {
        id: id.into(),
        name: "bash".into(),
        input: serde_json::json!({ "command": command }),
    }
}

async fn collect(client: Arc<dyn ChatStream>) -> (SessionState, Vec<SessionEvent>) {
    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "round-lifecycle",
        resolve_agent("act").unwrap(),
        Config {
            model: "test/model".into(),
            ..Config::default()
        },
        client,
        dir.path().to_path_buf(),
    );
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&events);
    run(&mut session, "go".into(), move |event| {
        sink.lock().unwrap().push(event);
    })
    .await
    .unwrap();
    let events = Arc::try_unwrap(events).unwrap().into_inner().unwrap();
    (session, events)
}

#[tokio::test]
async fn each_model_message_gets_one_round_covering_all_its_tools() {
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![completed(
                "",
                vec![bash("a", "printf a"), bash("b", "printf b")],
            )])
            .push_script(vec![
                LlmEvent::TextDelta("done".into()),
                completed("done", vec![]),
            ]),
    );
    let (session, events) = collect(mock).await;

    let starts: Vec<usize> = events
        .iter()
        .enumerate()
        .filter_map(|(i, e)| matches!(e, SessionEvent::LlmRoundStart { .. }).then_some(i))
        .collect();
    let ends: Vec<usize> = events
        .iter()
        .enumerate()
        .filter_map(|(i, e)| matches!(e, SessionEvent::LlmRoundEnd).then_some(i))
        .collect();
    let tool_ends: Vec<usize> = events
        .iter()
        .enumerate()
        .filter_map(|(i, e)| matches!(e, SessionEvent::ToolEnd { .. }).then_some(i))
        .collect();

    assert_eq!(starts.len(), 2, "two assistant messages mean two rounds");
    assert_eq!(ends.len(), 2, "every started round must end exactly once");
    assert_eq!(tool_ends.len(), 2, "the first round executes both tools");
    assert!(starts[0] < tool_ends[0]);
    assert!(tool_ends.iter().all(|i| *i < ends[0]));
    assert!(
        ends[0] < starts[1],
        "the next round starts from a reset state"
    );
    assert!(starts[1] < ends[1]);

    assert!(
        session
            .messages
            .iter()
            .all(|message| !message.text().contains("[turn cost")),
        "display timing must never enter model messages/context"
    );
}

#[tokio::test]
async fn terminal_text_round_ends_before_done() {
    let mock = Arc::new(MockChatClient::new().push_script(vec![completed("done", vec![])]));
    let (_, events) = collect(mock).await;
    let end = events
        .iter()
        .position(|e| matches!(e, SessionEvent::LlmRoundEnd))
        .expect("round end");
    let done = events
        .iter()
        .position(|e| matches!(e, SessionEvent::Done))
        .expect("task terminal event");
    assert!(end < done, "terminal tasks must hide turn cost before Done");
}

/// `Usage` carried by a `Completed` event, exercising the LlmUsage emission.
fn usage(total: u64) -> Option<Usage> {
    Some(Usage {
        input_tokens: total / 2,
        output_tokens: total - total / 2,
        total_tokens: total,
        ..Default::default()
    })
}

#[tokio::test]
async fn usage_round_emits_llm_usage_between_persist_and_round_end() {
    let mock = Arc::new(MockChatClient::new().push_script(vec![LlmEvent::Completed {
        text: "done".into(),
        tool_calls: vec![],
        usage: usage(1_234),
    }]));
    let (session, events) = collect(mock).await;

    let llm_usage = events
        .iter()
        .position(|e| {
            matches!(
                e,
                SessionEvent::LlmUsage {
                    total_tokens: 1234,
                    input_tokens: 617,
                    output_tokens: 617,
                }
            )
        })
        .expect("LlmUsage carries total plus the input/output split");
    let round_end = events
        .iter()
        .position(|e| matches!(e, SessionEvent::LlmRoundEnd))
        .expect("round end");
    let done = events
        .iter()
        .position(|e| matches!(e, SessionEvent::Done))
        .expect("done");
    assert!(
        llm_usage < round_end && round_end < done,
        "LlmUsage fires right after the assistant message is persisted, before the round closes"
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, SessionEvent::LlmUsage { .. }))
            .count(),
        1,
        "exactly one LlmUsage per assistant message with usage"
    );
    assert!(
        session
            .messages
            .iter()
            .any(|m| m.usage.total_tokens == 1_234),
        "usage is persisted on the assistant message"
    );
}

#[tokio::test]
async fn usage_less_rounds_emit_no_llm_usage() {
    // Providers that omit usage (and the default mock scripts) must not emit
    // LlmUsage — the TUI accumulator then simply sees no data for that round.
    let mock = Arc::new(MockChatClient::new().push_script(vec![completed("done", vec![])]));
    let (_, events) = collect(mock).await;
    assert!(
        events
            .iter()
            .all(|e| !matches!(e, SessionEvent::LlmUsage { .. })),
        "no usage on the wire means no LlmUsage event"
    );
}
