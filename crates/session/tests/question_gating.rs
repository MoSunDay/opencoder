//! End-to-end question-tool gating at the LLM-request boundary
//! (`runner::llm_call` visibility): the tools array a real `ChatRequest`
//! carries must follow the latent-gating contract.
//!
//! - act WITHOUT a task-plan skill: `question` absent.
//! - act WITH a task-plan body naming the skill in its first 500 chars: present.
//! - act WITH a review body: still absent (question is task-plan-only).
//! - sandbox: hidden unless a task-plan body is active (no agent exemptions).

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
async fn act_with_review_skill_hides_question() {
    // The built-in review skill no longer owns the question tool; only
    // task-plan unlocks it. A review body naming itself (and even the tool)
    // must leave the tool hidden.
    let body = "# review\n\nEvidence-driven review; use `question` for blocking ambiguities.";
    let (session, mock) = session_for("q-gate-review", "act", Some(body.into()));
    let names = requested_tools(session, mock).await;
    assert!(
        !names.contains(&"question".to_string()),
        "a review body must NOT unlock question for act, got: {names:?}"
    );
}

#[tokio::test]
async fn sandbox_without_skill_hides_question() {
    // No agent exemptions: the sandbox allowlist still carries `question`,
    // but without an active task-plan body the tool is not injected.
    let (session, mock) = session_for("q-gate-sandbox", "sandbox", None);
    let names = requested_tools(session, mock).await;
    assert!(
        !names.contains(&"question".to_string()),
        "sandbox without a skill must NOT see question, got: {names:?}"
    );
}

#[tokio::test]
async fn sandbox_with_plan_skill_sees_question() {
    let body = "# task-plan\n\n## Overview\n\nplan the launch; ask via question when blocked.";
    let (session, mock) = session_for("q-gate-sandbox-plan", "sandbox", Some(body.into()));
    let names = requested_tools(session, mock).await;
    assert!(
        names.contains(&"question".to_string()),
        "sandbox with an active task-plan body must see question, got: {names:?}"
    );
}

/// Contract bridge: the REAL seeded assets must match the gating logic.
/// Activating the built-in `task-plan` skill unlocks `question` for act;
/// activating the built-in `review` skill must not (its clarification
/// protocol is lookup-first + assumptions, never the interactive tool).
#[tokio::test]
async fn builtin_seed_assets_match_question_gating() {
    let root = tempfile::tempdir().unwrap();
    opencoder_core::seed_builtin_skills_in(root.path()).expect("seed");
    for (skill, unlocks) in [("task-plan", true), ("review", false)] {
        let path = root.path().join(skill).join("SKILL.md");
        let parsed =
            opencoder_core::skill::parse_skill(&path, "fallback").expect("seeded skill parses");
        // Mirror the runner: the unlock derives from the injected body.
        let injected = opencoder_core::body_with_source(&parsed);
        let (session, mock) = session_for(
            &format!("q-gate-seed-{skill}"),
            "act",
            Some(injected.clone()),
        );
        let names = requested_tools(session, mock).await;
        assert_eq!(
            names.contains(&"question".to_string()),
            unlocks,
            "built-in {skill} body question-visibility mismatch, got: {names:?}"
        );
    }
}
