//! Integration tests for queueable control commands (/act, /sandbox,
//! /act_clear_context, legacy /clear_context alias).
//!
//! Contracts:
//! - idle_short_circuit: a bare "/sandbox" prompt switches agent with NO LLM
//!   call
//! - queue_drains_control_cmds: a queue of ["/sandbox", "real prompt", "/act"]
//!   is fully drained FIFO in a single run — leading/trailing control
//!   commands are applied without LLM turns and the real prompt gets a
//!   turn; the run finishes (Done) with an empty queue
//! - clear_context_survives_resume: after /clear_context, resume
//!   reconstructs the fresh-start marker transcript. ClearContext ALWAYS
//!   preserves a chain: the last assistant reply becomes a neutral continuity
//!   seed, only a transcript with no assistant text collapses to the blank
//!   fresh-start sentinel. A clear from a sandbox session converges to act;
//!   an already-act session keeps its agent and its exact event sequence.

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config, ContentBlock, Message, Role};
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

/// Idle short-circuit: "/sandbox" switches agent immediately with zero LLM calls.
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
    run(&mut session, "/sandbox".into(), move |ev| {
        ev_clone.lock().unwrap().push(ev)
    })
    .await
    .unwrap();

    // Scope the MutexGuard so it is dropped before the `.await` below
    // (clippy::await_holding_lock). Block scope is the canonical fix; an
    // explicit drop() is not reliably recognized by the lint.
    {
        let evs = events.lock().unwrap();
        assert_eq!(session.agent.name, "sandbox", "agent switched to sandbox");
        assert!(
            evs.iter()
                .any(|e| matches!(e, SessionEvent::AgentSwitch(a) if a == "sandbox")),
            "AgentSwitch(sandbox) emitted"
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
    assert_eq!(meta.agent.as_deref(), Some("sandbox"), "/sandbox persists");
}

/// Queue drain: ["/sandbox", "do work", "/act"] is fully drained FIFO in a
/// single run — leading/trailing control commands are applied without LLM
/// turns, the real prompt gets a turn, and the run finishes (Done) with an
/// empty queue.
#[tokio::test]
async fn queue_drains_control_cmds_between_real_prompts() {
    let store = mem_store().await;
    seed(&store, "drain-sess", "act").await;

    // The mock: kickoff turn (done), then "do work" turn (done).
    // /sandbox and /act are applied without LLM calls.
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

    // Queue: /sandbox, "do work", /act
    store
        .admit_input(&mk_input("drain-sess", Delivery::Queue, "/sandbox"))
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
    // /sandbox (no LLM), "do work" (LLM turn), then at the next idle boundary
    // /act (no LLM) is applied and the queue empties -> Done.
    let agent_switches: Vec<&str> = evs
        .iter()
        .filter_map(|e| match e {
            SessionEvent::AgentSwitch(a) => Some(a.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        agent_switches.contains(&"sandbox"),
        "/sandbox applied -> AgentSwitch(sandbox)"
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

/// ClearContext survives resume: the seed marker is reconstructed and the
/// active agent is KEPT (no forced act switch).
#[tokio::test]
async fn clear_context_survives_resume() {
    let store = mem_store().await;
    seed(&store, "clear-sess", "sandbox").await;

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

    // ClearContext with a preserved result now EXECUTES it — push a mock
    // response for the execution turn.
    let mock =
        Arc::new(MockChatClient::new().push_script(vec![done_turn("done")])) as Arc<dyn ChatStream>;
    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "clear-sess",
        resolve_agent("sandbox").unwrap(),
        config(),
        mock,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();
    session.messages = msgs.clone();

    // The seed path continues running: one scripted follow-up turn (the model
    // sees the continuity context).
    let mock =
        Arc::new(MockChatClient::new().push_script(vec![done_turn("done")])) as Arc<dyn ChatStream>;
    session.client = mock;
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev_clone = events.clone();
    run(&mut session, "/clear_context".into(), move |ev| {
        ev_clone.lock().unwrap().push(ev)
    })
    .await
    .unwrap();

    // Scope the MutexGuard so it is dropped before the resume `.await` below
    // (clippy::await_holding_lock).
    {
        let evs = events.lock().unwrap();
        // After ClearContext + LLM turn: [seed marker, assistant_response]
        assert_eq!(
            session.messages.len(),
            2,
            "transcript = seed marker + assistant response"
        );
        assert_eq!(
            session.agent.name, "act",
            "sandbox clear converges to act"
        );
        assert!(session.handoff_seq.is_some(), "handoff_seq set");
        // The last assistant reply ("old answer") was preserved as the seed.
        assert_eq!(
            session.handoff_plan.as_deref(),
            Some("<<OPENCODER_CLEAR_SEED>>old answer")
        );
        assert!(
            evs.iter()
                .any(|e| matches!(e, SessionEvent::TranscriptReset(_))),
            "TranscriptReset emitted"
        );
        // Converged sandbox -> act: AgentSwitch(act) fires after the reset.
        let reset_idx = evs
            .iter()
            .position(|e| matches!(e, SessionEvent::TranscriptReset(_)))
            .expect("TranscriptReset emitted");
        let switch_idx = evs
            .iter()
            .position(
                |e| matches!(e, SessionEvent::AgentSwitch(a) if a == "act"),
            )
            .expect("AgentSwitch(act) emitted on sandbox clear");
        assert!(
            switch_idx > reset_idx,
            "AgentSwitch(act) must follow TranscriptReset, got {evs:?}"
        );
        assert!(
            evs.iter().any(|e| matches!(e, SessionEvent::Done)),
            "seed path falls through to an LLM turn (run completes)"
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
    // [reconstructed seed marker, assistant response]
    assert_eq!(
        resumed.messages.len(),
        2,
        "resume reconstructs seed marker + assistant response"
    );
    assert_eq!(
        resumed.agent.name, "act",
        "resume keeps the converged act agent"
    );
    let marker_text = resumed.messages[0].text();
    assert!(
        marker_text.contains("old answer"),
        "marker text carries the preserved reply: {marker_text}"
    );
    assert!(
        marker_text.starts_with("[Context cleared."),
        "seed marker uses the neutral continuity wrapper: {marker_text}"
    );
}

/// ClearContext with a preserved reply falls through to an LLM turn: the
/// model sees the continuity context (neutral wrapper + preserved text),
/// exactly once, and the raw command string never leaks.
#[tokio::test]
async fn clear_context_seed_falls_through_to_llm_turn() {
    let store = mem_store().await;
    seed(&store, "exec-sess", "sandbox").await;

    let msgs = vec![Message::user("u1", "implement feature X"), {
        let mut m = Message::assistant("a1");
        m.blocks
            .push(ContentBlock::text("I will implement X by..."));
        m
    }];
    store.append_messages("exec-sess", &msgs).await.unwrap();

    let mock: Arc<MockChatClient> =
        Arc::new(MockChatClient::new().push_script(vec![done_turn("done")]));
    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "exec-sess",
        resolve_agent("sandbox").unwrap(),
        config(),
        mock.clone() as Arc<dyn ChatStream>,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();
    session.messages = msgs.clone();

    run(&mut session, "/clear_context".into(), |_| {})
        .await
        .unwrap();

    // The LLM was called exactly once (the seed falls through to a turn).
    let requests = mock.requests();
    assert_eq!(
        requests.len(),
        1,
        "one LLM call for the continuity-context turn"
    );

    // The preserved reply appears in the model context, wrapped in the
    // neutral continuity framing (prior context, NOT a new instruction).
    let body = requests[0].to_body().to_string();
    assert!(
        body.contains("I will implement X by..."),
        "preserved reply must appear in the model context: {body}"
    );
    assert!(
        body.contains("preserved as continuity context"),
        "neutral seed wrapper must reach the model: {body}"
    );
    assert!(
        !body.contains("Execute it now"),
        "the autopilot handoff directive must NOT be used for a clear seed: {body}"
    );

    // The raw command string must NOT leak to the model.
    assert!(
        !body.contains("/clear_context"),
        "raw command string must not reach the model: {body}"
    );
}

/// ClearContext with no assistant text falls back to a blank fresh-start that
/// survives resume: resume reconstructs the sentinel fresh-start marker.
/// Exercises the canonical `/act_clear_context` spelling with no assistant
/// text — it must collapse to the blank fresh-start sentinel exactly like the
/// legacy `/clear_context` alias (persisted inputs keep working).
#[tokio::test]
async fn clear_context_no_assistant_text_survives_resume() {
    let store = mem_store().await;
    seed(&store, "clear-noplan", "sandbox").await;

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
        resolve_agent("sandbox").unwrap(),
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
        assert_eq!(
            session.agent.name, "act",
            "sandbox clear converges to act"
        );
        assert!(session.handoff_seq.is_some(), "handoff_seq set");
        // No assistant text -> blank sentinel stored so resume reconstructs
        // the fresh-start marker.
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
            evs.iter()
                .any(|e| matches!(e, SessionEvent::AgentSwitch(a) if a == "act")),
            "sandbox clear emits AgentSwitch(act), got {evs:?}"
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
    assert_eq!(
        resumed.agent.name, "act",
        "resume keeps the converged act agent"
    );
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
    );
    let client: Arc<dyn ChatStream> = mock.clone();

    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "steer-sess",
        resolve_agent("act").unwrap(),
        config(),
        client,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();

    // Steer "/sandbox" during the kickoff turn.
    store
        .admit_input(&mk_input("steer-sess", Delivery::Steer, "/sandbox"))
        .await
        .unwrap();

    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev_clone = events.clone();
    run(&mut session, "kickoff".into(), move |ev| {
        ev_clone.lock().unwrap().push(ev)
    })
    .await
    .unwrap();

    // "/sandbox" must NOT appear as a recorded user message.
    let user_texts: Vec<String> = session
        .messages
        .iter()
        .filter(|m| m.role == opencoder_core::Role::User)
        .map(|m| m.text())
        .collect();
    assert!(
        !user_texts.iter().any(|t| t.contains("/sandbox")),
        "/sandbox must not leak as user text: {:?}",
        user_texts
    );
    assert_eq!(
        session.agent.name, "sandbox",
        "steered /sandbox switched agent"
    );

    // The bare steered control command is the whole intent: AgentSwitch is
    // emitted, the run goes Done, and NO LLM turn is consumed.
    {
        let evs = events.lock().unwrap();
        assert!(
            evs.iter()
                .any(|e| matches!(e, SessionEvent::AgentSwitch(a) if a == "sandbox")),
            "AgentSwitch(sandbox) emitted"
        );
        assert!(
            evs.iter().any(|e| matches!(e, SessionEvent::Done)),
            "Done emitted"
        );
        assert!(
            !evs.iter().any(|e| matches!(e, SessionEvent::TextDelta(_))),
            "no LLM text streamed"
        );
    }
    assert!(
        mock.requests().is_empty(),
        "bare steered control command must not consume an LLM turn"
    );
}

/// After /clear_context the internal sentinel must never reach the model:
/// the fresh-start marker is what travels, and no LLM request body contains
/// the raw `<<OPENCODER_CLEAR_CONTEXT_MARKER>>` string (model context is
/// rebuilt from messages only, never from handoff_plan metadata).
#[tokio::test]
async fn clear_context_sentinel_never_reaches_model_context() {
    let store = mem_store().await;
    seed(&store, "sentinel-sess", "sandbox").await;
    store
        .append_messages("sentinel-sess", &[Message::user("u1", "old question")])
        .await
        .unwrap();

    let mock: Arc<MockChatClient> =
        Arc::new(MockChatClient::new().push_script(vec![done_turn("post-clear reply")]));

    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "sentinel-sess",
        resolve_agent("sandbox").unwrap(),
        config(),
        mock.clone() as Arc<dyn ChatStream>,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();
    session.messages = vec![Message::user("u1", "old question")];

    // Clear the context (idle short-circuit: no LLM call).
    run(&mut session, "/clear_context".into(), |_| {})
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

/// `/clear_context review` submitted as the idle prompt: clears context
/// AND runs "review" as a real prompt in the fresh context. The trailing
/// argument is recorded as a user message, not leaked as the raw command
/// string.
#[tokio::test]
async fn clear_context_compound_runs_rest_as_prompt() {
    let store = mem_store().await;
    seed(&store, "clear-compound", "act").await;

    // Pre-populate some messages so there is history to clear.
    let msgs = vec![Message::user("u1", "old question")];
    store
        .append_messages("clear-compound", &msgs)
        .await
        .unwrap();

    let mock = Arc::new(MockChatClient::new().push_script(vec![done_turn("fresh reply")]))
        as Arc<dyn ChatStream>;
    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "clear-compound",
        resolve_agent("act").unwrap(),
        config(),
        mock,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();
    session.messages = msgs.clone();

    run(&mut session, "/clear_context review".into(), |_| {})
        .await
        .unwrap();

    // Context was cleared and "review" recorded + executed; the agent is kept.
    assert_eq!(
        session.agent.name, "act",
        "agent unchanged by clear_context"
    );
    // "review" was recorded as a real user prompt.
    let has_review = session
        .messages
        .iter()
        .any(|m| m.role == Role::User && m.text().contains("review") && !m.synthetic);
    assert!(
        has_review,
        "trailing arg 'review' recorded as a real user prompt"
    );
    // The raw command must not leak as user text.
    let user_texts: Vec<String> = session
        .messages
        .iter()
        .filter(|m| m.role == Role::User)
        .map(|m| m.text())
        .collect();
    assert!(
        !user_texts.iter().any(|t| t.contains("/clear_context")),
        "raw command must not leak as user text: {:?}",
        user_texts
    );
    // Exactly one assistant turn (the "review" prompt execution).
    let assistant_turns = session
        .messages
        .iter()
        .filter(|m| m.role == Role::Assistant)
        .count();
    assert_eq!(assistant_turns, 1, "one assistant turn for 'review'");
}

/// Anti-fabrication regression: a plain assistant answer survives a clear as
/// a NEUTRAL seed (seed marker persisted, continuity wrapper in the message)
/// — it is NEVER repackaged as an execution directive, and the agent is kept.
#[tokio::test]
async fn clear_context_seeds_last_say_never_directive() {
    let store = mem_store().await;
    seed(&store, "clear-act", "act").await;

    let msgs = vec![Message::user("u1", "do task X"), {
        let mut m = Message::assistant("a1");
        m.blocks.push(ContentBlock::text("task done"));
        m.agent = Some("act".into());
        m
    }];
    store.append_messages("clear-act", &msgs).await.unwrap();

    // The seed path continues running: one scripted follow-up turn.
    let mock = Arc::new(MockChatClient::new().push_script(vec![done_turn("seeded reply")]))
        as Arc<dyn ChatStream>;
    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "clear-act",
        resolve_agent("act").unwrap(),
        config(),
        mock,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();
    session.messages = msgs.clone();

    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev_clone = events.clone();
    run(&mut session, "/clear_context".into(), move |ev| {
        ev_clone.lock().unwrap().push(ev)
    })
    .await
    .unwrap();

    {
        let evs = events.lock().unwrap();
        assert_eq!(
            session.handoff_plan.as_deref(),
            Some("<<OPENCODER_CLEAR_SEED>>task done"),
            "the last say is preserved as a seed, never as a directive"
        );
        assert_eq!(
            session.agent.name, "act",
            "ClearContext keeps the active agent"
        );
        // The synthetic seed message is neutral: no execution directive leaks.
        let seed_body = session.messages[0].text();
        assert!(
            seed_body.contains("prior context, not a new instruction"),
            "seed message must be neutral continuity context: {seed_body}"
        );
        assert!(
            !seed_body.contains("Execute it now"),
            "the autopilot handoff directive must never be synthesized here: {seed_body}"
        );
        assert!(
            !evs.iter()
                .any(|e| matches!(e, SessionEvent::AgentSwitch(_))),
            "no AgentSwitch on clear_context, got {evs:?}"
        );
    }

}
