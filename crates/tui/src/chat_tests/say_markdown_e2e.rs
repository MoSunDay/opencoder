//! E2E: full worker pipeline (runner → forwarder → UiEvents) folded into a
//! ChatView must leave every Say markdown-rendered after TurnDone.
use super::*;
use crate::worker::{UiCmd, UiEvent};
use opencoder_session::SessionState;

use std::sync::Arc;
use tokio::sync::mpsc;

async fn collect_turn(
    cmd_tx: mpsc::Sender<UiCmd>,
    evt_rx: &mut mpsc::Receiver<UiEvent>,
) -> Vec<UiEvent> {
    let _ = cmd_tx;
    let mut out = Vec::new();
    while let Some(ev) = evt_rx.recv().await {
        let done = matches!(ev, UiEvent::TurnDone(_));
        out.push(ev);
        if done {
            break;
        }
    }
    out
}

#[tokio::test]
async fn final_say_is_markdown_rendered_after_turn_done() {
    use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient, Usage};

    let dir = std::env::temp_dir().join(format!("say-md-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("note.txt");
    std::fs::write(&file, "content\n").unwrap();

    // Round 1: interim speech + tool. Round 2: interim speech + tool.
    // Round 3: final markdown answer, streamed in chunks.
    let client = MockChatClient::new()
        .push_script(vec![
            LlmEvent::TextDelta("checking the file first\n".into()),
            LlmEvent::Completed {
                text: "checking the file first\n".into(),
                tool_calls: vec![CompletedToolCall {
                    id: "t1".into(),
                    name: "read".into(),
                    input: serde_json::json!({"path": file.to_string_lossy()}),
                }],
                usage: Some(Usage::default()),
            },
        ])
        .push_script(vec![
            LlmEvent::TextDelta("## interim status\n".into()),
            LlmEvent::Completed {
                text: "## interim status\n".into(),
                tool_calls: vec![CompletedToolCall {
                    id: "t2".into(),
                    name: "read".into(),
                    input: serde_json::json!({"path": file.to_string_lossy()}),
                }],
                usage: Some(Usage::default()),
            },
        ])
        .push_script(vec![
            LlmEvent::TextDelta("**bold answer** part one\n".into()),
            LlmEvent::TextDelta("\n# Final Heading\n".into()),
            LlmEvent::TextDelta("tail line\n".into()),
            LlmEvent::Completed {
                text: "**bold answer** part one\n\n# Final Heading\ntail line\n".into(),
                tool_calls: vec![],
                usage: Some(Usage::default()),
            },
        ]);

    let mut sess = SessionState::new(
        "say-md-e2e",
        opencoder_core::resolve_agent("act").unwrap(),
        opencoder_core::Config {
            model: "m/g".into(),
            ..Default::default()
        },
        Arc::new(client) as Arc<dyn ChatStream>,
        dir.clone(),
    );

    let (evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(4); // tiny: force shedding
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(4);
    let worker = tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            if crate::worker::process_cmd(cmd, &mut sess, &evt_tx).await {
                break;
            }
        }
    });
    cmd_tx
        .send(UiCmd::Prompt("go".into(), Vec::new()))
        .await
        .unwrap();
    let events = collect_turn(cmd_tx, &mut evt_rx).await;
    worker.abort();

    assert!(
        events.iter().any(|e| matches!(e, UiEvent::TurnDone(_))),
        "turn must finish, got {events:?}"
    );

    // Fold exactly like the app loop does.
    let mut chat = ChatView::default();
    chat.begin_turn();
    for ev in events {
        match ev {
            UiEvent::Session(sev) => chat.apply(&sev),
            UiEvent::AssistantFinal(text) => chat.reconcile_completed_assistant(&text),
            UiEvent::TurnDone(_) => chat.finalize_assistant(),
        }
    }

    let text: Vec<String> = chat
        .flatten()
        .into_iter()
        .map(|l| l.spans.iter().map(|s| s.content.clone()).collect())
        .collect();
    let joined = text.join("\n");
    assert!(
        !joined.contains("**bold answer**"),
        "final say must be markdown-rendered, still raw:\n{joined}"
    );
    assert!(
        !joined.contains("# Final Heading"),
        "heading must be markdown-rendered, still raw:\n{joined}"
    );
    for (i, b) in chat.blocks.iter().enumerate() {
        if let ChatBlock::Assistant { raw, done, .. } = b {
            assert!(done, "assistant block {i} still open: {raw:?}");
        }
    }
}
