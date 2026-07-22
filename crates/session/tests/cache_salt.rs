//! Per-agent prefix-cache salt integration tests.
//!
//! The session runner derives a salt `<agent_name>:<session_id>` and stamps it
//! onto every outbound LLM request (both the `ChatRequest.cache_salt` field and
//! the serialized request body) when `config.cache_salt == Some(true)`, omits
//! it when disabled, and gives each subagent its own independent salt derived
//! from its child session id (`sub-<ULID>`). These tests exercise the runner
//! end-to-end against the shared mock so the salt is asserted on the real
//! `chat_stream` requests rather than the pure derivation function.

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::{run, SessionState};

// ---------------------------------------------------------------------------
// event builders
// ---------------------------------------------------------------------------

/// A single Completed turn with no tool calls, so the run loop stops after one
/// chat_stream request. Mirrors the `completed_no_tools` helper shape in
/// `interleaved_thinking.rs`.
fn completed_no_tools(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.to_string(),
        tool_calls: Vec::new(),
        usage: Some(Usage {
            input_tokens: 10,
            output_tokens: 20,
            total_tokens: 30,
            ..Default::default()
        }),
    }
}

/// Parent turn that dispatches an `explore` subagent via the `task` tool.
/// Mirrors the `task_turn` builder in `subagent.rs` /
/// `subagent_interleaved_thinking.rs` exactly (same tool name + input shape).
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
            ..Default::default()
        }),
    }
}

// ---------------------------------------------------------------------------
// session helpers
// ---------------------------------------------------------------------------

/// Build an `act` session with id `test-session`. Mirrors the `session_with`
/// helper in `interleaved_thinking.rs`.
async fn session_with(
    config: Config,
    client: Arc<dyn ChatStream>,
) -> (tempfile::TempDir, SessionState) {
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let s = SessionState::new(
        "test-session",
        agent,
        config,
        client,
        dir.path().to_path_buf(),
    );
    (dir, s)
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

/// Default config (`cache_salt == Some(true)`): the single outbound request
/// must carry `cache_salt = "act:test-session"` on both the typed field and the
/// serialized body.
#[tokio::test]
async fn main_agent_request_carries_session_salt_when_enabled() {
    let mock = Arc::new(MockChatClient::new().push_script(vec![completed_no_tools("done")]));
    let client: Arc<dyn ChatStream> = mock.clone();
    let (_dir, mut s) = session_with(Config::default(), client).await;

    run(&mut s, "hi".into(), |_| {}).await.unwrap();

    let reqs = mock.requests();
    assert!(!reqs.is_empty(), "expected at least one request");
    assert_eq!(
        reqs[0].cache_salt.as_deref(),
        Some("act:test-session"),
        "enabled salt must be <agent_name>:<session_id>"
    );
    // The salt must round-trip through the serialized request body.
    assert_eq!(
        reqs[0].to_body()["cache_salt"],
        serde_json::json!("act:test-session"),
        "cache_salt must appear as a top-level body field"
    );
}

/// With `cache_salt: Some(false)` the salt is omitted entirely: the typed field
/// is `None` and the body has no `cache_salt` key.
#[tokio::test]
async fn salt_omitted_when_disabled() {
    let config = Config {
        cache_salt: Some(false),
        ..Config::default()
    };
    let mock = Arc::new(MockChatClient::new().push_script(vec![completed_no_tools("done")]));
    let client: Arc<dyn ChatStream> = mock.clone();
    let (_dir, mut s) = session_with(config, client).await;

    run(&mut s, "hi".into(), |_| {}).await.unwrap();

    let reqs = mock.requests();
    assert!(!reqs.is_empty());
    assert_eq!(
        reqs[0].cache_salt, None,
        "disabled salt must be absent from the ChatRequest"
    );
    assert!(
        reqs[0].to_body().get("cache_salt").is_none(),
        "disabled salt must NOT appear in the serialized body"
    );
}

/// A subagent dispatches a child session whose salt is derived from its OWN
/// `<agent_name>:<session_id>` (`explore:sub-<ULID>`), independent of the
/// parent. The shared mock records parent + child `chat_stream` calls in order,
/// so we can assert both salts from the same request log.
#[tokio::test]
async fn subagent_request_carries_its_own_salt() {
    //   script[0] parent turn 1 -> task tool call (dispatch explore child)
    //   script[1] child  turn 1 -> plain text done
    //   script[2] parent turn 2 -> plain text done
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![task_turn("explore the repo")])
            .push_script(vec![completed_no_tools("found things")])
            .push_script(vec![completed_no_tools("all done")]),
    );
    let client: Arc<dyn ChatStream> = mock.clone();
    let (_dir, mut s) = session_with(Config::default(), client).await;

    run(&mut s, "delegate".into(), |_| {}).await.unwrap();

    let reqs = mock.requests();
    assert!(
        reqs.len() >= 3,
        "expected >=3 requests (2 parent + 1 child), got {}",
        reqs.len()
    );

    // The parent's salt is `act:test-session`.
    let has_parent_salt = reqs
        .iter()
        .any(|r| r.cache_salt.as_deref() == Some("act:test-session"));
    assert!(
        has_parent_salt,
        "parent request salt must be act:test-session; salts: {:?}",
        reqs.iter()
            .map(|r| r.cache_salt.clone())
            .collect::<Vec<_>>()
    );

    // The child's salt is `explore:sub-<ULID>`; the ULID is random, so
    // prefix-match the whole namespace.
    let has_child_salt = reqs.iter().any(|r| {
        r.cache_salt
            .as_deref()
            .is_some_and(|salt| salt.starts_with("explore:sub-"))
    });
    assert!(
        has_child_salt,
        "explore subagent must carry its own salt starting with explore:sub-; \
         salts: {:?}",
        reqs.iter()
            .map(|r| r.cache_salt.clone())
            .collect::<Vec<_>>()
    );
}
