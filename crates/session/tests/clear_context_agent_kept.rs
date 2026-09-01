//! Plan-to-act handoff contract for `/act_clear_context` (legacy
//! `/clear_context`): preserve the newest plan as an execution directive,
//! switch to act, and execute it through idle, queue, steer, and compound
//! entry paths.

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config, ContentBlock, Message, Role};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient, Usage};
use opencoder_session::{resume, run, SessionEvent, SessionState};
use opencoder_store::{Delivery, LibsqlStore, SessionInput, Store};

const SENTINEL: &str = "<<OPENCODER_CLEAR_CONTEXT_MARKER>>";

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

/// Assistant reply that makes a transcript seed-flavoured (the newest
/// non-empty say is preserved as the continuity seed).
fn assistant_say(id: &str, text: &str) -> Message {
    let mut m = Message::assistant(id);
    m.blocks.push(ContentBlock::text(text));
    m
}

fn plan_session(
    id: &str,
    mock: Arc<MockChatClient>,
    store: &Arc<dyn Store>,
) -> SessionState {
    let dir = tempfile::tempdir().unwrap();
    SessionState::new(
        id,
        resolve_agent("plan").unwrap(),
        config(),
        mock,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created()
}

fn user_texts(session: &SessionState) -> Vec<String> {
    session
        .messages
        .iter()
        .filter(|m| m.role == Role::User)
        .map(|m| m.text())
        .collect()
}

fn assert_reset_then_act(evs: &[SessionEvent]) {
    let reset = evs
        .iter()
        .position(|e| matches!(e, SessionEvent::TranscriptReset(_)))
        .expect("TranscriptReset emitted");
    let switch = evs
        .iter()
        .position(|e| matches!(e, SessionEvent::AgentSwitch(name) if name == "act"))
        .expect("AgentSwitch(act) emitted");
    assert!(reset < switch, "reset must precede act switch: {evs:?}");
}

/// (a) Idle bare `/act_clear_context` on a plan session with a preserved
/// plan: switches to act, executes the directive in one LLM turn, and resume
/// keeps the focused act transcript.
#[tokio::test]
async fn plan_idle_bare_clear_hands_off_and_persists() {
    let store = mem_store().await;
    seed(&store, "keep-clear", "plan").await;
    let msgs = vec![Message::user("u1", "old question"), assistant_say("a1", "old answer")];
    store.append_messages("keep-clear", &msgs).await.unwrap();

    let mock = Arc::new(MockChatClient::new().push_script(vec![done_turn("seeded reply")]));
    let mut session = plan_session("keep-clear", mock.clone(), &store);
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
        assert_eq!(session.agent.name, "act", "clear converges to act");
        assert_eq!(
            mock.call_count(),
            1,
            "exactly the seed execution turn ran"
        );
        assert_reset_then_act(&evs);
        assert!(evs.iter().any(|e| matches!(e, SessionEvent::Done)));
    }
    assert!(
        session.messages[0].text().contains("Execute it now"),
        "plan becomes an execution directive"
    );
    assert_eq!(
        session.handoff_plan.as_deref(),
        Some("old answer")
    );
    let meta = store.get_session("keep-clear").await.unwrap().unwrap();
    assert_eq!(meta.agent.as_deref(), Some("act"));

    // Resume rebuilds directive + reply only under act.
    let resumed = resume(
        store.clone(),
        "keep-clear",
        config(),
        Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>,
        session.working_dir.clone(),
    )
    .await
    .unwrap();
    assert_eq!(resumed.agent.name, "act");
    assert_eq!(resumed.messages.len(), 2, "directive + assistant response");
    assert!(resumed.messages[0].text().contains("Execute it now"));
}

/// (b) Plan session with no assistant content: the clear degrades to the
/// blank fresh-start sentinel and stops without an LLM call, but still
/// converges to act so the next request is writable.
#[tokio::test]
async fn plan_sentinel_clear_stops_without_llm() {
    let store = mem_store().await;
    seed(&store, "keep-blank", "plan").await;
    let msgs = vec![
        Message::user("u1", "old question"),
        Message::user("u2", "another question"),
    ];
    store
        .append_messages("keep-blank", &msgs)
        .await
        .unwrap();

    let mock = Arc::new(MockChatClient::new());
    let mut session = plan_session("keep-blank", mock.clone(), &store);
    session.messages = msgs.clone();

    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev_clone = events.clone();
    run(&mut session, "/clear_context".into(), move |ev| {
        ev_clone.lock().unwrap().push(ev)
    })
    .await
    .unwrap();

    let evs = events.lock().unwrap();
    assert_eq!(session.agent.name, "act", "blank clear still converges to act");
    assert_eq!(mock.call_count(), 0, "sentinel stops without an LLM call");
    assert_eq!(
        session.handoff_plan.as_deref(),
        Some(SENTINEL),
        "no assistant content -> blank fresh-start sentinel"
    );
    assert_reset_then_act(&evs);
}

/// (c) A queued bare clear between real prompts applies at the idle
/// boundary: the plan directive executes under act before the later real
/// prompt.
#[tokio::test]
async fn plan_queue_drain_clears_before_real_prompt() {
    let store = mem_store().await;
    seed(&store, "keep-queue", "plan").await;
    let msgs = vec![Message::user("u1", "old question"), assistant_say("a1", "old answer")];
    store
        .append_messages("keep-queue", &msgs)
        .await
        .unwrap();

    // Turns: kickoff, seed execution, real prompt.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![done_turn("kickoff reply")])
            .push_script(vec![done_turn("seeded reply")])
            .push_script(vec![done_turn("work done")]),
    );
    let mut session = plan_session("keep-queue", mock.clone(), &store);
    session.messages = msgs.clone();

    store
        .admit_input(&mk_input("keep-queue", Delivery::Queue, "/act_clear_context"))
        .await
        .unwrap();
    let _seq = store
        .admit_input(&mk_input("keep-queue", Delivery::Queue, "real prompt"))
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
        assert_eq!(session.agent.name, "act");
        assert_reset_then_act(&evs);
    }
    let still_pending = store
        .pending_inputs("keep-queue", Delivery::Queue)
        .await
        .unwrap();
    assert!(still_pending.is_empty(), "queue fully drained");

    // Three turns ran (kickoff, seed execution, real prompt); the real-prompt
    // request carries the prompt as a user message under the fresh context.
    let requests = mock.requests();
    assert_eq!(requests.len(), 3, "kickoff + seed execution + real prompt");
    let last = requests[2].to_body().to_string();
    assert!(
        last.contains("real prompt"),
        "the real prompt must reach the model after the clear: {last}"
    );
}

/// (d) A steered `/clear_context` is absorbed at the turn boundary (steer
/// claimed at the top of the loop, before any LLM call): the plan switches to
/// act, the command never leaks as user text, and the preserved directive owns
/// exactly one LLM turn.
#[tokio::test]
async fn plan_steer_clear_hands_off_and_executes() {
    let store = mem_store().await;
    seed(&store, "keep-steer", "plan").await;
    let msgs = vec![Message::user("u1", "old question"), assistant_say("a1", "old answer")];
    store
        .append_messages("keep-steer", &msgs)
        .await
        .unwrap();

    let mock = Arc::new(MockChatClient::new().push_script(vec![done_turn("executed")]));
    let mut session = plan_session("keep-steer", mock.clone(), &store);
    session.messages = msgs.clone();

    // Steer admitted before the run: claimed at run_loop's first turn
    // boundary, so its synthetic directive becomes the turn input.
    store
        .admit_input(&mk_input("keep-steer", Delivery::Steer, "/clear_context"))
        .await
        .unwrap();

    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev_clone = events.clone();
    run(&mut session, "kickoff".into(), move |ev| {
        ev_clone.lock().unwrap().push(ev)
    })
    .await
    .unwrap();

    let evs = events.lock().unwrap();
    assert_eq!(session.agent.name, "act");
    assert_reset_then_act(&evs);
    assert!(
        evs.iter().any(|e| matches!(e, SessionEvent::Done)),
        "Done emitted"
    );
    assert!(
        !user_texts(&session)
            .iter()
            .any(|t| t.contains("/clear_context")),
        "steered command must not leak as user text: {:?}",
        user_texts(&session)
    );
    assert_eq!(
        mock.call_count(),
        1,
        "steered handoff executes the preserved plan"
    );
}

/// (e) Compound `/act_clear_context review` on a plan session: the clear
/// switches to act and the rest runs beside the plan directive in one LLM
/// call; the raw command never leaks as user text.
#[tokio::test]
async fn plan_compound_clear_runs_rest_under_act() {
    let store = mem_store().await;
    seed(&store, "keep-compound", "plan").await;
    let msgs = vec![Message::user("u1", "old question"), assistant_say("a1", "old answer")];
    store
        .append_messages("keep-compound", &msgs)
        .await
        .unwrap();

    let mock = Arc::new(MockChatClient::new().push_script(vec![done_turn("review done")]));
    let mut session = plan_session("keep-compound", mock.clone(), &store);
    session.messages = msgs.clone();

    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev_clone = events.clone();
    run(&mut session, "/act_clear_context review".into(), move |ev| {
        ev_clone.lock().unwrap().push(ev)
    })
    .await
    .unwrap();

    {
        let evs = events.lock().unwrap();
        assert_eq!(session.agent.name, "act");
        assert_reset_then_act(&evs);
    }
    assert_eq!(mock.call_count(), 1, "the rest ran as one real prompt");
    assert!(
        session
            .messages
            .iter()
            .any(|m| m.role == Role::User && m.text().contains("review") && !m.synthetic),
        "'review' recorded as a real user prompt"
    );
    assert!(
        !user_texts(&session)
            .iter()
            .any(|t| t.contains("/act_clear_context")),
        "raw command must not leak as user text: {:?}",
        user_texts(&session)
    );
}
