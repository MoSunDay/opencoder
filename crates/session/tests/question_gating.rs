//! End-to-end question-tool gating at the LLM-request boundary
//! (`runner::llm_call` visibility): the tools array a real `ChatRequest`
//! carries must follow the latent-gating contract.
//!
//! - act WITHOUT a task-plan/review skill: `question` absent.
//! - act WITH a skill body naming the skill in its first 500 chars: present.
//! - sandbox: present regardless (base-prompt clarification protocol).

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{LlmEvent, MockChatClient, Usage};
use opencoder_session::SessionState;

fn config() -> Config {
    Config {
        model: "m/g".into(),
        ..Config::default()
    }
}

fn done_turn() -> LlmEvent {
    LlmEvent::Completed {
        text: "ok".into(),
        tool_calls: vec![],
        usage: Some(Usage::default()),
    }
}

fn session_for(
    id: &str,
    agent: &str,
    skill_body: Option<String>,
) -> (SessionState, Arc<MockChatClient>) {
    let mock = Arc::new(MockChatClient::new().push_script(vec![done_turn()]));
    let dir = tempfile::tempdir().unwrap();
    let session = SessionState::new(
        id,
        resolve_agent(agent).unwrap(),
        config(),
        mock.clone(),
        dir.path().to_path_buf(),
    );
    if let Some(body) = skill_body {
        session.set_skill(Some(body));
    }
    (session, mock)
}

async fn requested_tools(session: SessionState, mock: Arc<MockChatClient>) -> Vec<String> {
    let mut inner = session;
    let mut events = Vec::new();
    opencoder_session::run(&mut inner, "do the thing".into(), |ev| events.push(ev))
        .await
        .unwrap();
    let reqs = mock.requests();
    assert_eq!(reqs.len(), 1, "exactly one LLM round");
    let mut names: Vec<String> = reqs[0]
        .tools
        .iter()
        .filter_map(|t| t["function"]["name"].as_str().map(str::to_string))
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn act_without_skill_hides_question() {
    let (session, mock) = session_for("q-gate-act", "act", None);
    let names = requested_tools(session, mock).await;
    assert!(
        !names.contains(&"question".to_string()),
        "act without the skill must not see question, got: {names:?}"
    );
}

#[tokio::test]
async fn act_with_plan_skill_sees_question() {
    let body =
        "# task-plan\n\n## Overview\n\nMulti-phase planning. Ask via `question` when blocked.";
    let (session, mock) = session_for("q-gate-plan", "act", Some(body.into()));
    let names = requested_tools(session, mock).await;
    assert!(
        names.contains(&"question".to_string()),
        "a task-plan body must unlock question for act, got: {names:?}"
    );
}

#[tokio::test]
async fn act_with_review_skill_sees_question() {
    let body = "# review\n\nEvidence-driven review; use `question` for blocking ambiguities.";
    let (session, mock) = session_for("q-gate-review", "act", Some(body.into()));
    let names = requested_tools(session, mock).await;
    assert!(
        names.contains(&"question".to_string()),
        "a review body must unlock question for act, got: {names:?}"
    );
}

#[tokio::test]
async fn sandbox_sees_question_without_any_skill() {
    let (session, mock) = session_for("q-gate-sandbox", "sandbox", None);
    let names = requested_tools(session, mock).await;
    assert!(
        names.contains(&"question".to_string()),
        "sandbox must always see question, got: {names:?}"
    );
}
