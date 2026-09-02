//! Headless submission contracts after the sandbox->plan rename (rules/01
//! regression tests). The reverted sandbox-mode interlude spelled the
//! read-only switch `/sandbox`; the canonical spelling is `/plan` again.
//! Pinned here:
//!   1. a legacy `/sandbox ...` submission is rewritten and reaches the runner
//!      as the plan switch;
//!   2. a bare legacy `/sandbox` switch stops without an LLM turn;
//!   3. a live `/plan $skill-or-text` compound reaches the runner as the
//!      plan switch and the trailing text still runs as the prompt.

use std::sync::{Arc, Mutex};

use opencoder_cli::run::rewrite_legacy_sandbox_prefix;
use opencoder_core::{resolve_agent, Config, Role};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_session::{run, SessionEvent, SessionState};
use opencoder_store::{LibsqlStore, SessionMeta, Store};

fn mock_client() -> Arc<dyn ChatStream> {
    Arc::new(
        MockChatClient::new().with_default(vec![LlmEvent::Completed {
            text: "ok".into(),
            tool_calls: vec![],
            usage: None,
        }]),
    )
}

/// Session wired to an in-memory store (mirrors the runner test fixtures) so
/// agent switches and message records persist exactly as in a real headless
/// run, deterministically and offline.
async fn make_session(id: &str) -> SessionState {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    store
        .create_session(&SessionMeta {
            id: id.into(),
            title: Some("test".into()),
            agent: Some("act".into()),
            model: Some("m/g".into()),

            autopilot_mode: None,
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
            summary_images: vec![],
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: None,
            requirement: None,
        })
        .await
        .unwrap();
    let agent = resolve_agent("act").unwrap();
    let config = Config {
        model: "m/g".into(),
        ..Default::default()
    };
    SessionState::new(id, agent, config, mock_client(), std::env::temp_dir()).with_store(store)
}

fn record_events() -> (
    Arc<Mutex<Vec<SessionEvent>>>,
    impl FnMut(SessionEvent) + Send,
) {
    let events: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    (events, move |ev: SessionEvent| {
        sink.lock().unwrap().push(ev);
    })
}

#[tokio::test]
async fn legacy_plan_compound_reaches_runner_as_plan_switch() {
    let mut session = make_session("legacy-plan-compound").await;
    // The regression shape: irregular spacing after the legacy token.
    let prompt = rewrite_legacy_sandbox_prefix("/sandbox  draft the plan");
    let (events, on_event) = record_events();
    run(&mut session, prompt, on_event).await.unwrap();

    assert_eq!(
        session.agent.name, "plan",
        "legacy /sandbox compound must land on the plan agent"
    );
    assert!(
        events
            .lock()
            .unwrap()
            .iter()
            .any(|ev| matches!(ev, SessionEvent::AgentSwitch(a) if a == "plan")),
        "runner must emit the plan AgentSwitch event"
    );
    // Compound: the trailing text executed as a real prompt (recorded user
    // message + the mock LLM reply), not swallowed by the switch.
    let user = session
        .messages
        .iter()
        .find(|m| m.role == Role::User)
        .expect("compound rest must be recorded as a user prompt");
    assert_eq!(user.text(), "draft the plan");
    assert!(
        session
            .messages
            .iter()
            .any(|m| m.role == Role::Assistant && m.text() == "ok"),
        "LLM turn must run for the compound rest"
    );
}

#[tokio::test]
async fn legacy_sandbox_bare_switch_stops_without_llm_turn() {
    let mut session = make_session("legacy-sandbox-bare").await;
    let prompt = rewrite_legacy_sandbox_prefix("/sandbox");
    let (events, on_event) = record_events();
    run(&mut session, prompt, on_event).await.unwrap();

    assert_eq!(session.agent.name, "plan");
    let evs = events.lock().unwrap();
    assert!(
        evs.iter()
            .any(|ev| matches!(ev, SessionEvent::AgentSwitch(a) if a == "plan")),
        "bare switch must emit AgentSwitch(plan)"
    );
    assert!(
        evs.iter().any(|ev| matches!(ev, SessionEvent::Done)),
        "bare switch short-circuits with Done"
    );
    assert!(
        session.messages.iter().all(|m| m.role != Role::User),
        "bare switch records no user prompt"
    );
}

#[tokio::test]
async fn plan_compound_switches_and_runs_rest() {
    let mut session = make_session("plan-compound").await;
    // Trailing text with an $skill token that matches nothing: the compound
    // must still switch and submit the rest (token stays literal).
    let (events, on_event) = record_events();
    run(
        &mut session,
        "/plan $no-such-skill-x review the diff".to_string(),
        on_event,
    )
    .await
    .unwrap();

    assert_eq!(session.agent.name, "plan");
    assert!(
        events
            .lock()
            .unwrap()
            .iter()
            .any(|ev| matches!(ev, SessionEvent::AgentSwitch(a) if a == "plan")),
        "headless output must see the plan switch banner event"
    );
    let user = session
        .messages
        .iter()
        .find(|m| m.role == Role::User)
        .expect("compound rest must be recorded as a user prompt");
    assert!(
        user.text().contains("review the diff"),
        "trailing text must survive the switch: {}",
        user.text()
    );
    assert!(
        session
            .messages
            .iter()
            .any(|m| m.role == Role::Assistant && m.text() == "ok"),
        "LLM turn must run for the compound rest"
    );
}
