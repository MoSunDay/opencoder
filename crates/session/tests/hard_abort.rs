//! Verifies the hard-abort path: a mid-tool cancellation stops a running bash
//! command immediately (kill_on_drop drops the tool future) and the run loop
//! returns promptly with a `Status("interrupted")` event.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use opencode_core::{resolve_agent, Config};
use opencode_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencode_session::{run, SessionEvent, SessionState};
use tokio_util::sync::CancellationToken;

fn bash_call(cmd: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: String::new(),
        tool_calls: vec![CompletedToolCall {
            id: "call_1".into(),
            name: "bash".into(),
            input: serde_json::json!({ "command": cmd }),
        }],
        usage: Some(Usage { input_tokens: 0, output_tokens: 0, total_tokens: 0 }),
    }
}

fn done_event() -> LlmEvent {
    LlmEvent::Completed {
        text: "done".into(),
        tool_calls: vec![],
        usage: Some(Usage { input_tokens: 0, output_tokens: 0, total_tokens: 0 }),
    }
}

#[tokio::test]
async fn cancel_hard_aborts_a_running_tool() {
    // Round 1 asks for a long bash command. The default script (reached only if
    // the abort FAILED to stop the loop) is a plain done.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![bash_call("sleep 5")])
            .with_default(vec![done_event()]),
    ) as Arc<dyn ChatStream>;
    let config = Config { model: "main/glm-5.2".into(), ..Config::default() };
    let agent = resolve_agent("act").unwrap();
    let cancel = CancellationToken::new();
    let mut s = SessionState::new(
        "hard-abort",
        agent,
        config,
        mock,
        std::env::temp_dir(),
    )
    .with_cancel(cancel.clone());

    let events: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let ev_clone = events.clone();

    let start = Instant::now();
    let handle = tokio::spawn(async move {
        run(&mut s, "go".into(), move |ev| {
            ev_clone.lock().unwrap().push(ev);
        })
        .await
    });

    // Let the LLM call resolve and the bash tool start, then cancel mid-tool.
    tokio::time::sleep(Duration::from_millis(300)).await;
    cancel.cancel();

    // If hard-abort works, the run returns well under the 5s sleep. If it does
    // not, the run blocks ~5s and the 8s outer timeout catches it.
    let outcome = tokio::time::timeout(Duration::from_secs(8), handle).await;
    let elapsed = start.elapsed();

    assert!(outcome.is_ok(), "run did not return within 8s; hard-abort is broken");
    assert!(
        elapsed < Duration::from_secs(3),
        "run took {elapsed:?}; expected a sub-3s abort (sleep 5 was supposed to be killed)"
    );

    let saw_interrupted = events
        .lock()
        .unwrap()
        .iter()
        .any(|ev| matches!(ev, SessionEvent::Status(msg) if msg == "interrupted"));
    assert!(saw_interrupted, "expected a Status(interrupted) event after cancel");
}
