//! Worker-level full flow for the question tool: a plan turn asks a question,
//! the shared hub gets resolved by the "UI side" mid-turn, and the answer
//! becomes the Tool result that drives the model's follow-up — the exact
//! contract `app.rs` relies on (dialog → `QuestionHub::resolve`).
//!
//! This mirrors the session-level test but through the TUI worker's
//! `process_cmd` boundary (UiCmd::Prompt), proving the plumbing survives the
//! worker task split (the worker owns the SessionState while awaiting).

use std::sync::Arc;
use std::time::Duration;

use opencoder_core::{resolve_agent, Config};
use opencoder_llm::tool_call::CompletedToolCall;
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient, Usage};
use opencoder_session::tools::question::QuestionHub;
use opencoder_session::{SessionEvent, SessionState};
use opencoder_tui::worker::{process_cmd, UiCmd, UiEvent};

fn question_turn(id: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: String::new(),
        tool_calls: vec![CompletedToolCall {
            id: id.into(),
            name: "question".into(),
            input: serde_json::json!({
                "question": "Which database should the plan target?",
                "options": ["sqlite", "postgres"]
            }),
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

#[tokio::test]
async fn worker_prompt_with_question_resolved_mid_turn() {
    let hub = QuestionHub::new();
    hub.attach(); // the TUI attaches before spawning the worker
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![question_turn("qw-1")])
            .push_script(vec![text_done("## Plan\nGoal: use postgres")]),
    );
    let dir = tempfile::tempdir().unwrap();
    let session = SessionState::new(
        "question-worker",
        resolve_agent("plan").unwrap(),
        Config {
            model: "m/g".into(),
            ..Config::default()
        },
        mock as Arc<dyn ChatStream>,
        dir.path().to_path_buf(),
    )
    .with_question_hub(hub.clone());

    let (tx, mut rx) = tokio::sync::mpsc::channel::<UiEvent>(64);
    // Drive the worker exactly like `run_app` does: one Prompt per turn.
    let worker = tokio::spawn(async move {
        let mut sess = session;
        process_cmd(
            UiCmd::Prompt("plan a migration".into(), vec![]),
            &mut sess,
            &tx,
        )
        .await;
        sess
    });

    // UI side: wait for the question to land on the hub (bounded), answer it.
    for _ in 0..2_000 {
        if hub.waiting_count() > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(hub.waiting_count() > 0, "worker never asked the question");
    assert!(
        hub.resolve("qw-1", "postgres".into()),
        "resolve while waiting"
    );

    let sess = tokio::time::timeout(Duration::from_secs(15), worker)
        .await
        .expect("turn completes after the answer")
        .unwrap();

    // The answer is the ToolResult block of the question call.
    let tool_results: Vec<&opencoder_core::ContentBlock> = sess
        .messages
        .iter()
        .flat_map(|m| m.blocks.iter())
        .filter(|b| matches!(b, opencoder_core::ContentBlock::ToolResult { tool_use_id, .. } if tool_use_id == "qw-1"))
        .collect();
    assert_eq!(
        tool_results.len(),
        1,
        "exactly one ToolResult for the question call"
    );
    match tool_results[0] {
        opencoder_core::ContentBlock::ToolResult {
            content, is_error, ..
        } => {
            assert!(!is_error);
            assert_eq!(content, "postgres");
        }
        _ => unreachable!(),
    }

    // The follow-up assistant text (same turn) reflects the answer context.
    assert!(
        sess.messages.iter().any(|m| m
            .blocks
            .iter()
            .any(|b| matches!(b, opencoder_core::ContentBlock::Text { text } if text.contains("postgres")))),
        "follow-up assistant text mentions the answered database"
    );

    // The UI event stream carried ToolStart/ToolEnd so the dialog could open.
    let mut saw_start = false;
    let mut saw_end = false;
    while let Ok(ev) = rx.try_recv() {
        match ev {
            UiEvent::Session(SessionEvent::ToolStart { name, .. }) if name == "question" => {
                saw_start = true
            }
            UiEvent::Session(SessionEvent::ToolEnd { name, output, .. }) if name == "question" => {
                saw_end = true;
                assert_eq!(output, "postgres");
            }
            _ => {}
        }
    }
    assert!(saw_start, "ToolStart(question) reached the UI channel");
    assert!(saw_end, "ToolEnd(question) reached the UI channel");
    assert_eq!(hub.waiting_count(), 0, "hub drained after the turn");
}
