//! Regression: compaction must reset the tool-failure / doom-loop / bash-timeout
//! guards.
//!
//! Before the fix, the three counters in `run_loop` (`doom`, `tool_failures`,
//! `bash_timeout_first`) were declared once before the loop and never cleared.
//! When compaction succeeded, stale tool-failure counts from pre-compaction
//! turns persisted, so a couple of post-compaction failures could trip the
//! guard (or pre-compaction doom signatures cause a false doom-loop abort).
//!
//! Setup: `max_consecutive_failures = 3`, `context_limit = 10_000`,
//! `compaction.budget = 10_000`, `reserved = 0`. Two tool failures accumulate
//! below the token budget; the second reports `input_tokens = 12_000` which
//! trips compaction on the next loop iteration. After compaction (with the fix)
//! the failure counter is cleared, so two more post-compaction failures (max
//! count 2) stay under the threshold of 3 and the run reaches `done()`.
//!
//! Without the fix the pre-compaction count (2) survives, the first
//! post-compaction failure pushes it to 3 → guard trips → run returns Err on
//! the 4th call.

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{tool_call::CompletedToolCall, ChatStream, LlmEvent, MockChatClient, Usage};
use opencoder_session::{run, SessionState};
use serde_json::json;

/// A call to an unregistered tool → always `is_error`. Each call carries a
/// unique `n` (so the doom-loop `name:input` signature differs every turn)
/// while the *name* stays constant so the per-name consecutive-failure counter
/// accumulates. `input_tokens` is the model-reported usage that drives
/// `should_compact` via `session.last_usage`.
fn failing_tool_call(n: u32, input_tokens: u64) -> LlmEvent {
    LlmEvent::Completed {
        text: String::new(),
        tool_calls: vec![CompletedToolCall {
            id: "c1".into(),
            name: "nonexistent_tool".into(),
            input: json!({ "n": n }),
        }],
        usage: Some(Usage {
            input_tokens,
            output_tokens: 0,
            total_tokens: input_tokens,
            ..Default::default()
        }),
    }
}

/// Compaction summary: a completed turn with no tool calls and tiny usage.
fn summary_event() -> LlmEvent {
    LlmEvent::Completed {
        text: "compacted".into(),
        tool_calls: vec![],
        usage: Some(Usage {
            input_tokens: 1,
            output_tokens: 1,
            total_tokens: 2,
            ..Default::default()
        }),
    }
}

fn done() -> LlmEvent {
    LlmEvent::Completed {
        text: "done".into(),
        tool_calls: vec![],
        usage: None,
    }
}

/// Tight budget so compaction fires after a single over-budget report.
fn compact_config() -> Config {
    let mut c = Config {
        model: "mock/test".into(),
        ..Config::default()
    };
    c.context_limit = Some(10_000);
    c.compaction.auto = true;
    c.compaction.context_threshold = 10_000;
    c.compaction.reserved = 0;
    c.tool_guard.max_consecutive_failures = 3;
    c.tool_guard.backoff_base_ms = 0;
    c.tool_guard.backoff_max_ms = 0;
    c
}

async fn make_session(config: Config, client: Arc<dyn ChatStream>) -> SessionState {
    // Persistent path (not a dropped TempDir) so real tools like `bash` can
    // `current_dir` into it when they spawn.
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    SessionState::new("test-session", agent, config, client, dir.keep())
}

#[tokio::test]
async fn compaction_resets_tool_failure_counter() {
    // FIFO script: the summarization call consumes its own entry too.
    let mock = Arc::new(
        MockChatClient::new()
            // below budget (5000 < 10000): failure 1, no compaction
            .push_script(vec![failing_tool_call(1, 5_000)])
            // over budget (12000 >= 10000): failure 2, triggers compaction
            .push_script(vec![failing_tool_call(2, 12_000)])
            // compaction summary call (consumed by `compact`)
            .push_script(vec![summary_event()])
            // post-compaction: failures 1 & 2 (under the 3 threshold)
            .push_script(vec![failing_tool_call(3, 100)])
            .push_script(vec![failing_tool_call(4, 100)])
            // clean finish — reached only because the counter was reset
            .push_script(vec![done()]),
    );
    let client: Arc<dyn ChatStream> = mock.clone();
    let mut s = make_session(compact_config(), client).await;

    let result = run(&mut s, "test".into(), |_| {}).await;

    // With the fix: post-compaction max count = 2 < 3 → run reaches done().
    assert!(
        result.is_ok(),
        "compaction should reset the failure counter; run should succeed"
    );
    // 5 turn calls + 1 compaction-summary call = 6 total chat_stream calls.
    assert_eq!(mock.call_count(), 6);
}
