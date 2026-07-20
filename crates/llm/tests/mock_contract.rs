//! MockChatClient contract: replays scripted events FIFO and records requests.

use opencoder_llm::{ChatRequest, ChatStream, CompletedToolCall, LlmEvent, MockChatClient};

fn req_with(model: &str) -> ChatRequest {
    ChatRequest {
        model: model.to_string(),
        messages: vec![],
        tools: vec![],
        tool_choice: None,
        temperature: None,
        max_tokens: None,
        reasoning_effort: None,
        cache_salt: None,
    }
}

async fn drain(rx: &mut tokio::sync::mpsc::Receiver<LlmEvent>) -> Vec<LlmEvent> {
    let mut out = Vec::new();
    while let Some(ev) = rx.recv().await {
        out.push(ev);
    }
    out
}

#[tokio::test]
async fn mock_replays_scripted_events_in_fifo_order() {
    let mock = MockChatClient::new()
        .push_script(vec![LlmEvent::TextDelta("a".into()), done_event("a")])
        .push_script(vec![LlmEvent::TextDelta("b".into()), done_event("b")]);

    let mut rx1 = mock.chat_stream(req_with("m")).unwrap();
    let ev1 = drain(&mut rx1).await;
    assert_eq!(ev1.len(), 2);
    assert!(matches!(ev1[0], LlmEvent::TextDelta(ref s) if s == "a"));

    let mut rx2 = mock.chat_stream(req_with("m")).unwrap();
    let ev2 = drain(&mut rx2).await;
    assert!(matches!(ev2[0], LlmEvent::TextDelta(ref s) if s == "b"));

    assert_eq!(mock.call_count(), 2);
}

#[tokio::test]
async fn mock_records_request_bodies_for_assertion() {
    let mock = MockChatClient::new().push_script(vec![done_event("x")]);
    let mut rx = mock.chat_stream(req_with("glm-5.2")).unwrap();
    let _ = drain(&mut rx).await;

    let reqs = mock.requests();
    assert_eq!(reqs.len(), 1);
    assert_eq!(
        reqs[0].model, "glm-5.2",
        "model must be captured for switch assertions"
    );
}

#[tokio::test]
async fn mock_default_script_repeats_when_queue_empty() {
    let mock = MockChatClient::new()
        .push_script(vec![done_event("first")])
        .with_default(vec![LlmEvent::TextDelta("loop".into()), done_event("loop")]);

    let mut rx1 = mock.chat_stream(req_with("m")).unwrap();
    let e1 = drain(&mut rx1).await;
    assert!(matches!(e1.last().unwrap(), LlmEvent::Completed { .. }));

    // queue empty → default kicks in
    let mut rx2 = mock.chat_stream(req_with("m")).unwrap();
    let e2 = drain(&mut rx2).await;
    assert_eq!(e2.len(), 2);

    let mut rx3 = mock.chat_stream(req_with("m")).unwrap();
    let e3 = drain(&mut rx3).await;
    assert_eq!(e3.len(), 2, "default keeps serving");
}

#[tokio::test]
async fn mock_errors_when_exhausted_and_no_default() {
    let mock = MockChatClient::new();
    let err = mock.chat_stream(req_with("m")).err();
    assert!(err.is_some(), "exhausted mock without default must error");
}

fn done_event(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.to_string(),
        tool_calls: Vec::<CompletedToolCall>::new(),
        usage: None,
    }
}
