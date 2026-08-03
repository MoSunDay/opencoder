//! Integration tests for queueable control commands (/act, /plan,
//! /act_clear_context).
//!
//! Contracts:
//! - idle_short_circuit: a bare "/plan" prompt switches mode with NO LLM call
//! - queue_drains_control_cmds: a queue of ["/plan", "real prompt", "/act"]
//!   is fully drained FIFO in a single run — leading/trailing control
//!   commands are applied without LLM turns and the real prompt gets a
//!   turn; the run finishes (Done) with an empty queue
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
        display_text: None,
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

/// Queue drain: ["/plan", "do work", "/act"] is fully drained FIFO in a
/// single run — leading/trailing control commands are applied without LLM
/// turns, the real prompt gets a turn, and the run finishes (Done) with an
/// empty queue.
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

    // Query the store BEFORE taking the events lock (avoids holding a
    // MutexGuard across an .await): the queue should be fully drained.
    let still_pending = store
        .pending_inputs("drain-sess", Delivery::Queue)
        .await
        .unwrap();

    let evs = events.lock().unwrap();

    // After "kickoff" turn -> idle, the queue drains FIFO in a single run:
    // /plan (no LLM), "do work" (LLM turn), then at the next idle boundary
    // /act (no LLM) is applied and the queue empties -> Done.
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
        "/act applied in the same run -> AgentSwitch(act)"
    );

    // The final agent should be act (trailing /act was drained).
    assert_eq!(session.agent.name, "act", "final agent is act");

    // QueueConsumed fires for all three items; the queue is now empty.
    let consumed_count = evs
        .iter()
        .filter(|e| matches!(e, SessionEvent::QueueConsumed { .. }))
        .count();
    assert_eq!(consumed_count, 3, "all three queue items consumed");
    assert!(still_pending.is_empty(), "queue fully drained");

    // Done should be emitted exactly once (at the very end of the run).
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
            "transcript collapsed to 1 handoff marker"
        );
        assert_eq!(session.agent.name, "act", "switched to act");
        assert!(session.handoff_seq.is_some(), "handoff_seq set");
        // The finalized plan ("old answer") was preserved, not blanked.
        assert_eq!(session.handoff_plan.as_deref(), Some("old answer"));
        assert!(
            evs.iter()
                .any(|e| matches!(e, SessionEvent::TranscriptReset(_))),
            "TranscriptReset emitted"
        );
        assert!(
            evs.iter()
                .any(|e| matches!(e, SessionEvent::PlanHandoff(p) if p == "old answer")),
            "PlanHandoff emitted carrying the preserved plan"
        );
    }

    // Now resume from the store and verify the handoff marker survives.
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
        "resume reconstructs single plan-handoff marker"
    );
    assert_eq!(resumed.agent.name, "act");
    let marker_text = resumed.messages[0].text();
    assert!(
        marker_text.contains("old answer"),
        "marker text carries the preserved plan: {marker_text}"
    );
}

/// ClearContext with no finalized plan falls back to a blank fresh-start that
/// survives resume: resume reconstructs the sentinel fresh-start marker.
#[tokio::test]
async fn clear_context_no_plan_survives_resume() {
    let store = mem_store().await;
    seed(&store, "clear-noplan", "plan").await;

    // Only user messages: no assistant plan text to hand off.
    let msgs = vec![
        Message::user("u1", "old question"),
        Message::user("u2", "another question"),
    ];
    store.append_messages("clear-noplan", &msgs).await.unwrap();

    let mock = Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>;
    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "clear-noplan",
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

    {
        let evs = events.lock().unwrap();
        assert_eq!(
            session.messages.len(),
            1,
            "transcript collapsed to 1 fresh-start marker"
        );
        assert_eq!(session.agent.name, "act", "switched to act");
        assert!(session.handoff_seq.is_some(), "handoff_seq set");
        // No plan -> blank sentinel stored so resume reconstructs fresh-start.
        // (CLEAR_CONTEXT_SENTINEL is pub(crate); assert the literal value.)
        assert_eq!(
            session.handoff_plan.as_deref(),
            Some("<<OPENCODER_CLEAR_CONTEXT_MARKER>>"),
        );
        assert!(
            session.messages[0].text().contains("Context cleared"),
            "marker is the blank fresh-start"
        );
        assert!(
            evs.iter()
                .any(|e| matches!(e, SessionEvent::TranscriptReset(_))),
            "TranscriptReset emitted"
        );
        assert!(
            !evs
                .iter()
                .any(|e| matches!(e, SessionEvent::PlanHandoff(_))),
            "no PlanHandoff when there is no plan"
        );
    }

    // Resume reconstructs the blank fresh-start marker.
    let resumed = resume(
        store.clone(),
        "clear-noplan",
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
        "marker text is the blank fresh-start: {marker_text}"
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

/// After /act_clear_context the internal sentinel must never reach the model:
/// the fresh-start marker is what travels, and no LLM request body contains
/// the raw `<<OPENCODER_CLEAR_CONTEXT_MARKER>>` string (model context is
/// rebuilt from messages only, never from handoff_plan metadata).
#[tokio::test]
async fn clear_context_sentinel_never_reaches_model_context() {
    let store = mem_store().await;
    seed(&store, "sentinel-sess", "plan").await;
    store
        .append_messages("sentinel-sess", &[Message::user("u1", "old question")])
        .await
        .unwrap();

    let mock: Arc<MockChatClient> =
        Arc::new(MockChatClient::new().push_script(vec![done_turn("post-clear reply")]));

    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "sentinel-sess",
        resolve_agent("plan").unwrap(),
        config(),
        mock.clone() as Arc<dyn ChatStream>,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();
    session.messages = vec![Message::user("u1", "old question")];

    // Clear the context (idle short-circuit: no LLM call).
    run(&mut session, "/act_clear_context".into(), |_| {})
        .await
        .unwrap();
    assert_eq!(session.messages.len(), 1, "transcript collapsed to marker");

    // A real prompt after the clear triggers exactly one LLM call.
    run(&mut session, "continue".into(), |_| {}).await.unwrap();

    let requests = mock.requests();
    assert_eq!(requests.len(), 1, "one LLM call after clear");
    let body = requests[0].to_body().to_string();
    assert!(
        !body.contains("<<OPENCODER_CLEAR_CONTEXT_MARKER>>"),
        "sentinel must never be stored into model context, got: {body}"
    );
    // The fresh-start marker is present in the model context (the first
    // message is the system prompt, so scan the user messages).
    let has_marker = requests[0]
        .messages
        .iter()
        .any(|m| m.to_string().contains("Context cleared"));
    assert!(
        has_marker,
        "fresh-start marker must lead the model context: {body}"
    );
}
