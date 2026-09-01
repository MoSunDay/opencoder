//! Integration test for the bare-steer short-circuit.
//!
//! Contract: a bare control command admitted as a STEER (e.g. "/plan" with
//! no trailing text) that is the ONLY input of a drain run must switch the
//! agent mode and go idle WITHOUT invoking the LLM. Before the fix, such a
//! steer fell through to `run_one_llm_call` with no new user message on the
//! transcript — a wasteful (and, against an empty `MockChatClient`, fatal)
//! LLM call. Mirrors the initial-prompt short-circuit in `run_with_registry`.

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{ChatStream, MockChatClient};
use opencoder_session::{run, SessionEvent, SessionState};
use opencoder_store::{Delivery, LibsqlStore, SessionInput, Store};

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

fn config() -> Config {
    Config {
        model: "m/g".into(),
        ..Config::default()
    }
}

async fn seed(store: &Arc<dyn Store>, id: &str, agent: &str) {
    store
        .create_session(&opencoder_store::SessionMeta {
            id: id.into(),
            agent: Some(agent.into()),
            model: Some("m/g".into()),
            created_at: 0,
            updated_at: 0,
            ..Default::default()
        })
        .await
        .unwrap();
}

fn mk_input(session_id: &str, delivery: Delivery, prompt: &str) -> SessionInput {
    SessionInput {
        seq: None,
        id: opencoder_session::runner::new_id(),
        session_id: session_id.into(),
        delivery,
        prompt: prompt.into(),
        images: vec![],
        display_text: None,
        admitted_seq: 0,
        promoted_seq: None,
    }
}

/// Bare "/plan" admitted as a steer, then drained with an empty initial
/// prompt, switches to the plan agent with ZERO LLM calls. Against an empty
/// `MockChatClient` (no pushed scripts) this is deterministic: without the
/// short-circuit the run reaches `run_one_llm_call` → empty mock → failure.
#[tokio::test]
async fn bare_steer_switches_mode_with_no_llm_call() {
    let store = mem_store().await;
    seed(&store, "bare-steer", "act").await;

    // No pushed scripts: any LLM call would fail/panic the run.
    let mock = Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>;
    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "bare-steer",
        resolve_agent("act").unwrap(),
        config(),
        mock,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();

    // The ONLY input is the bare steer "/plan"; an empty initial prompt
    // forces drain mode, where the steer is claimed at the top of run_loop.
    store
        .admit_input(&mk_input("bare-steer", Delivery::Steer, "/plan"))
        .await
        .unwrap();

    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev_clone = events.clone();
    run(&mut session, String::new(), move |ev| {
        ev_clone.lock().unwrap().push(ev)
    })
    .await
    .unwrap();

    // Scope the MutexGuard so it is dropped before any `.await`
    // (clippy::await_holding_lock).
    {
        let evs = events.lock().unwrap();
        // Mode switched to plan.
        assert_eq!(session.agent.name, "plan", "agent switched to plan");
        assert!(
            evs.iter()
                .any(|e| matches!(e, SessionEvent::AgentSwitch(a) if a == "plan")),
            "AgentSwitch(plan) emitted"
        );
        // Run completed.
        assert!(
            evs.iter().any(|e| matches!(e, SessionEvent::Done)),
            "Done emitted"
        );
        // No LLM call happened: no text streamed.
        assert!(
            !evs.iter().any(|e| matches!(e, SessionEvent::TextDelta(_))),
            "no LLM text streamed"
        );
    }

    // Persisted to store.
    let meta = store.get_session("bare-steer").await.unwrap().unwrap();
    assert_eq!(meta.agent.as_deref(), Some("plan"));
}

/// A compound "/plan review" admitted as a steer applies the mode switch at
/// the turn boundary and records "review" as a real prompt: exactly one LLM
/// call executes the rest in the new mode, and the raw command never leaks
/// into the transcript.
#[tokio::test]
async fn steered_compound_plan_switches_then_runs_rest() {
    let store = mem_store().await;
    seed(&store, "steer-compound", "act").await;

    // One scripted LLM call: the "review" rest recorded after the switch.
    let mock = Arc::new(MockChatClient::new().push_script(vec![done_turn("reviewed")]));
    let client: Arc<dyn ChatStream> = mock.clone();
    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "steer-compound",
        resolve_agent("act").unwrap(),
        config(),
        client,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();

    store
        .admit_input(&mk_input(
            "steer-compound",
            Delivery::Steer,
            "/plan review",
        ))
        .await
        .unwrap();

    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev_clone = events.clone();
    run(&mut session, "kickoff".into(), move |ev| {
        ev_clone.lock().unwrap().push(ev)
    })
    .await
    .unwrap();

    {
        let evs = events.lock().unwrap();
        assert_eq!(session.agent.name, "plan", "agent switched to plan");
        assert!(
            evs.iter()
                .any(|e| matches!(e, SessionEvent::AgentSwitch(a) if a == "plan")),
            "AgentSwitch(plan) emitted"
        );
        assert!(
            evs.iter().any(|e| matches!(e, SessionEvent::Done)),
            "Done emitted"
        );
    }

    // The raw command must not leak; "review" is the recorded prompt in the
    // new mode.
    let user_texts: Vec<String> = session
        .messages
        .iter()
        .filter(|m| m.role == opencoder_core::Role::User)
        .map(|m| m.text())
        .collect();
    assert!(
        !user_texts.iter().any(|t| t.contains("/plan")),
        "/plan must not leak as user text: {:?}",
        user_texts
    );
    assert!(
        user_texts.iter().any(|t| t.contains("review")),
        "compound rest must be recorded as the prompt: {:?}",
        user_texts
    );
    // Exactly one LLM call: the "review" turn in plan mode.
    assert_eq!(
        mock.requests().len(),
        1,
        "one LLM call for the compound rest"
    );
    let meta = store.get_session("steer-compound").await.unwrap().unwrap();
    assert_eq!(meta.agent.as_deref(), Some("plan"));
}

fn done_turn(text: &str) -> opencoder_llm::LlmEvent {
    opencoder_llm::LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![],
        usage: None,
    }
}
