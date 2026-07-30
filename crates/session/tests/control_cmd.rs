//! Integration tests for queueable control commands (/act, /plan,
//! /act_clear_context).
//!
//! Contracts:
//! - idle_short_circuit: a bare "/plan" prompt switches mode with NO LLM call
//! - queue_drains_control_cmds: a queue of ["/plan", "real prompt", "/act"]
//!   applies the control commands without LLM turns and runs the real prompt
//! - clear_context_survives_resume: after /act_clear_context, resume
//!   reconstructs the fresh-start marker transcript

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config, ContentBlock, Message};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient, Usage};
use opencoder_session::{resume, run, SessionEvent, SessionState};
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

fn done_turn(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![],
        usage: Some(Usage::default()),
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
        admitted_seq: 0,
        promoted_seq: None,
    }
}

/// Idle short-circuit: "/plan" switches mode immediately with zero LLM calls.
#[tokio::test]
async fn idle_short_circuit_switches_with_no_llm_call() {
    let store = mem_store().await;
    seed(&store, "idle-sess", "act").await;

    let mock = Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>;
    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "idle-sess",
        resolve_agent("act").unwrap(),
        config(),
        mock,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();

    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev_clone = events.clone();
    run(&mut session, "/plan".into(), move |ev| {
        ev_clone.lock().unwrap().push(ev)
    })
    .await
    .unwrap();

    // Scope the MutexGuard so it is dropped before the `.await` below
    // (clippy::await_holding_lock). Block scope is the canonical fix; an
    // explicit drop() is not reliably recognized by the lint.
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
        // No LLM call happened: no TextDelta, no ToolStart.
        assert!(
            !evs.iter().any(|e| matches!(e, SessionEvent::TextDelta(_))),
            "no LLM text streamed"
        );
    }

    // Persisted to store.
    let meta = store.get_session("idle-sess").await.unwrap().unwrap();
    assert_eq!(meta.agent.as_deref(), Some("plan"));
}

/// Queue drain: ["/plan", "do work", "/act"] applies control commands without
/// LLM turns and processes the real prompt.
#[tokio::test]
async fn queue_drains_control_cmds_between_real_prompts() {
    let store = mem_store().await;
    seed(&store, "drain-sess", "act").await;

    // The mock: kickoff turn (done), then "do work" turn (done).
    // /plan and /act are applied without LLM calls.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![done_turn("kickoff reply")])
            .push_script(vec![done_turn("work done")]),
    ) as Arc<dyn ChatStream>;

    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "drain-sess",
        resolve_agent("act").unwrap(),
        config(),
        mock,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();

    // Queue: /plan, "do work", /act
    store
        .admit_input(&mk_input("drain-sess", Delivery::Queue, "/plan"))
        .await
        .unwrap();
    store
        .admit_input(&mk_input("drain-sess", Delivery::Queue, "do work"))
        .await
        .unwrap();
    let _act_seq = store
        .admit_input(&mk_input("drain-sess", Delivery::Queue, "/act"))
        .await
        .unwrap();

    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev_clone = events.clone();
    // Kick off with a real prompt so the loop starts.
    run(&mut session, "kickoff".into(), move |ev| {
        ev_clone.lock().unwrap().push(ev)
    })
    .await
    .unwrap();

    let evs = events.lock().unwrap();

    // After "kickoff" turn (bash -> tool exec -> idle), the queue drains:
    // /plan (no LLM), "do work" (LLM turn), then at the next idle /act (no LLM).
    let agent_switches: Vec<&str> = evs
        .iter()
        .filter_map(|e| match e {
            SessionEvent::AgentSwitch(a) => Some(a.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        agent_switches.contains(&"plan"),
        "/plan applied -> AgentSwitch(plan)"
    );
    assert!(
        agent_switches.contains(&"act"),
        "/act applied -> AgentSwitch(act)"
    );

    // The final agent should be act.
    assert_eq!(session.agent.name, "act", "final agent is act");

    // QueueConsumed should fire for all 3 queued items.
    let consumed_count = evs
        .iter()
        .filter(|e| matches!(e, SessionEvent::QueueConsumed { .. }))
        .count();
    assert_eq!(consumed_count, 3, "all 3 queue items consumed");

    // Done should be emitted exactly once (at the very end).
    let done_count = evs
        .iter()
        .filter(|e| matches!(e, SessionEvent::Done))
        .count();
    assert_eq!(done_count, 1, "Done emitted exactly once");
}

/// ClearContext survives resume: the fresh-start marker is reconstructed.
#[tokio::test]
async fn clear_context_survives_resume() {
    let store = mem_store().await;
    seed(&store, "clear-sess", "plan").await;

    // Pre-populate with some messages in the store.
    let msgs = vec![
        Message::user("u1", "old question"),
        {
            let mut m = Message::assistant("a1");
            m.blocks.push(ContentBlock::text("old answer"));
            m
        },
        Message::user("u2", "another question"),
    ];
    store.append_messages("clear-sess", &msgs).await.unwrap();

    let mock = Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>;
    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "clear-sess",
        resolve_agent("plan").unwrap(),
        config(),
        mock,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();
    session.messages = msgs.clone();

    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev_clone = events.clone();
    run(&mut session, "/act_clear_context".into(), move |ev| {
        ev_clone.lock().unwrap().push(ev)
    })
    .await
    .unwrap();

    // Scope the MutexGuard so it is dropped before the resume `.await` below
    // (clippy::await_holding_lock).
    {
        let evs = events.lock().unwrap();
        assert_eq!(
            session.messages.len(),
            1,
            "transcript collapsed to 1 marker"
        );
        assert_eq!(session.agent.name, "act", "switched to act");
        assert!(session.handoff_seq.is_some(), "handoff_seq set");
        assert!(
            evs.iter()
                .any(|e| matches!(e, SessionEvent::TranscriptReset(_))),
            "TranscriptReset emitted"
        );
    }

    // Now resume from the store and verify the marker survives.
    let resumed = resume(
        store.clone(),
        "clear-sess",
        config(),
        Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>,
        dir.path().to_path_buf(),
    )
    .await
    .unwrap();
    assert_eq!(
        resumed.messages.len(),
        1,
        "resume reconstructs single fresh-start marker"
    );
    assert_eq!(resumed.agent.name, "act");
    let marker_text = resumed.messages[0].text();
    assert!(
        marker_text.contains("Context cleared"),
        "marker text preserved: {marker_text}"
    );
}

/// Steering a control command applies it immediately without recording it as
/// user text (defensive intercept).
#[tokio::test]
async fn steered_control_cmd_not_recorded_as_user_text() {
    let store = mem_store().await;
    seed(&store, "steer-sess", "act").await;

    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![done_turn("reply")])
            .push_script(vec![done_turn("after steer")]),
    ) as Arc<dyn ChatStream>;

    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "steer-sess",
        resolve_agent("act").unwrap(),
        config(),
        mock,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();

    // Steer "/plan" during the kickoff turn.
    store
        .admit_input(&mk_input("steer-sess", Delivery::Steer, "/plan"))
        .await
        .unwrap();

    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev_clone = events.clone();
    run(&mut session, "kickoff".into(), move |ev| {
        ev_clone.lock().unwrap().push(ev)
    })
    .await
    .unwrap();

    // "/plan" must NOT appear as a recorded user message.
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
    assert_eq!(session.agent.name, "plan", "steered /plan switched agent");
}
