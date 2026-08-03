//! Regression: a Tab-queued `{$skill} + text` submission keeps its skill
//! through the drain (the report: "queued 也是如果 skill插入+ 其他需求，
//! skill 插入信息会消失").
//!
//! The TUI admits only the token-stripped clean text to the store (the LLM
//! must never see the token), activating the skill in-memory + persisting
//! `sessions.skill` at queue time (`resolve_persist`, see skill_persist.rs).
//! This test pins the full chain through the worker: queue admit → submit →
//! idle-boundary drain → the drained turn's system prompt carries the skill
//! body, and the queued user message is the clean text.
use std::sync::Arc;

use opencoder_core::{message::now_ms, resolve_agent, Config};
use opencoder_llm::{LlmEvent, MockChatClient};
use opencoder_session::SessionState;
use opencoder_store::{Delivery, LibsqlStore, SessionInput, SessionMeta, SessionPatch, Store};
use opencoder_tui::worker::{process_cmd, UiCmd, UiEvent};
use tokio::sync::mpsc;

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

fn text_done(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![],
        usage: None,
    }
}

/// Extract the system message content from a ChatRequest's messages.
fn system_content(req: &opencoder_llm::ChatRequest) -> String {
    req.messages
        .iter()
        .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("system"))
        .and_then(|m| m.get("content").and_then(|c| c.as_str()))
        .unwrap_or("")
        .to_string()
}

#[tokio::test]
async fn queued_combined_submission_drains_with_skill() {
    let store = mem_store().await;
    store
        .create_session(&SessionMeta {
            id: "q-skill".into(),
            agent: Some("act".into()),
            ..Default::default()
        })
        .await
        .unwrap();

    // Kickoff turn settles; the drained queued follow-up is the 2nd call.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![text_done("t1")])
            .push_script(vec![text_done("t2")]),
    );
    let (tx, _rx) = mpsc::channel::<UiEvent>(64);
    let mut sess = SessionState::new(
        "q-skill",
        resolve_agent("act").expect("act agent"),
        Config::default(),
        mock.clone(),
        std::env::temp_dir(),
    )
    .with_store(store.clone());

    // Mirror `KeyAction::Queue`'s `resolve_persist` on `{$haiku} fix the bug`:
    // activate the skill through the shared Arc (same handle the worker's
    // `run_one_llm_call` reads) and persist `sessions.skill`, then admit only
    // the token-stripped clean text.
    let skill_body = "Always answer in haiku form.";
    let skill_handle = sess.skill_prompt.clone();
    *skill_handle.lock().unwrap() = Some(skill_body.to_string());
    store
        .update_session(
            "q-skill",
            &SessionPatch {
                skill: Some(skill_body.to_string()),
                updated_at: Some(now_ms()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    store
        .admit_input(&SessionInput {
            seq: None,
            id: "q-1".into(),
            session_id: "q-skill".into(),
            delivery: Delivery::Queue,
            prompt: "fix the bug".into(),
            images: Vec::new(),
            display_text: None,
            admitted_seq: 0,
            promoted_seq: None,
        })
        .await
        .unwrap();

    // Submit the kickoff; the run drains the queued follow-up at the first
    // idle boundary (no tool calls in turn 1).
    let quit = process_cmd(UiCmd::Prompt("kickoff".into(), vec![]), &mut sess, &tx).await;
    assert!(!quit, "Prompt must not break the worker loop");

    let requests = mock.requests();
    assert!(
        requests.len() >= 2,
        "expected kickoff turn + drained queued follow-up, got {}",
        requests.len()
    );

    // Effect: the drained turn's system prompt carries the skill body.
    let drained_system = system_content(&requests[1]);
    assert!(
        drained_system.contains("haiku"),
        "drained queued turn must run with the skill in the system prompt: {drained_system}"
    );

    // The queued user message is the clean text (token stripped at admit —
    // the store/LLM never see the token).
    let user_msgs: Vec<&serde_json::Value> = requests[1]
        .messages
        .iter()
        .filter(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
        .collect();
    assert!(
        user_msgs.iter().any(|m| {
            m.get("content")
                .and_then(|c| c.as_str())
                .is_some_and(|c| c.contains("fix the bug"))
        }),
        "queued clean text must reach the model: {user_msgs:?}"
    );
    assert!(
        user_msgs.iter().all(|m| {
            !m.get("content")
                .and_then(|c| c.as_str())
                .is_some_and(|c| c.contains("{$"))
        }),
        "the {{$skill}} token must never reach the LLM: {user_msgs:?}"
    );
}
