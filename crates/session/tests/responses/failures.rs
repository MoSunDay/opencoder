use super::wire::*;
use opencoder_core::resolve_agent;
use opencoder_session::{run, SessionEvent, SessionState};
use serde_json::json;
use std::sync::Arc;

#[tokio::test]
async fn incomplete_response_never_executes_even_a_complete_tool_item() {
    let dir = tempfile::tempdir().unwrap();
    let _home = opencoder_core::scoped_config_home(dir.path().to_path_buf());
    std::fs::write(dir.path().join("keep.txt"), "original").unwrap();
    let tool = call(
        "edit1",
        "edit",
        json!({"path":"keep.txt","old_string":"original","new_string":"changed"}),
    );
    let server=serve(vec![Reply::events(vec![
        json!({"type":"response.output_item.done","output_index":0,"item":tool}),
        json!({"type":"response.incomplete","response":{"incomplete_details":{"reason":"max_output_tokens"}}})
    ])]).await;
    let cfg = config(&server.url);
    let mut session = SessionState::new(
        "failed",
        resolve_agent("act").unwrap(),
        cfg.clone(),
        Arc::new(client(&cfg)),
        dir.path().to_path_buf(),
    );
    let mut events = Vec::new();
    let error = run(&mut session, "update".into(), |e| events.push(e))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("max_output_tokens"));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("keep.txt")).unwrap(),
        "original"
    );
    assert!(!events
        .iter()
        .any(|e| matches!(e, SessionEvent::ToolStart { .. })));
    assert!(!session.messages.iter().any(|m| m.provider_state.is_some()));
}

#[tokio::test]
async fn retry_discards_attempt_state_and_emits_reset_before_fresh_text() {
    let dir = tempfile::tempdir().unwrap();
    let _home = opencoder_core::scoped_config_home(dir.path().to_path_buf());
    let server=serve(vec![Reply::events(vec![json!({"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"discard"})]),Reply::events(vec![completed(vec![answer("fresh")])])]).await;
    let cfg = config(&server.url);
    let mut session = SessionState::new(
        "retry",
        resolve_agent("act").unwrap(),
        cfg.clone(),
        Arc::new(client(&cfg)),
        dir.path().to_path_buf(),
    );
    let mut events = Vec::new();
    run(&mut session, "hello".into(), |e| events.push(e))
        .await
        .unwrap();
    let reset = events
        .iter()
        .position(|e| matches!(e, SessionEvent::LlmAttemptReset))
        .unwrap();
    assert!(events[reset + 1..]
        .iter()
        .any(|e| matches!(e,SessionEvent::TextDelta(t)if t=="fresh")));
    assert_eq!(session.messages.last().unwrap().text(), "fresh");
    assert!(!serde_json::to_string(session.messages.last().unwrap())
        .unwrap()
        .contains("discard"));
}
