//! Worker-level integration tests for `/clear_context` (alias
//! `/act_clear_context`): the fold must preserve the newest assistant reply
//! as a neutral continuity seed, execute it in exactly one LLM turn, persist
//! a resume boundary, and leave an act session's agent untouched. From plan,
//! the preserved reply becomes an execution directive and the agent switches
//! to act before that turn runs.

use std::sync::Arc;
use std::time::Duration;

use opencoder_core::{resolve_agent, Config, ContentBlock, Message, Role};
use opencoder_llm::{LlmEvent, MockChatClient};
use opencoder_session::{
    is_clear_context_handoff, is_clear_context_seed, resume, SessionEvent, SessionState,
};
use opencoder_store::{LibsqlStore, SessionMeta, Store};
use opencoder_tui::worker::{process_cmd, UiCmd, UiEvent};
use tokio::sync::mpsc;

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

fn assistant_with_text(id: &str, text: &str) -> Message {
    let mut m = Message::assistant(id);
    m.blocks.push(ContentBlock::text(text));
    m
}

fn text_done(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![],
        usage: None,
    }
}

fn act_session(id: &str, mock: Arc<MockChatClient>, store: Arc<dyn Store>) -> SessionState {
    SessionState::new(
        id,
        resolve_agent("act").expect("act agent"),
        Config {
            model: "m/g".into(),
            ..Config::default()
        },
        mock as Arc<dyn opencoder_llm::ChatStream>,
        std::env::temp_dir(),
    )
    .with_store(store)
    .mark_session_created()
}

fn plan_session(id: &str, mock: Arc<MockChatClient>, store: Arc<dyn Store>) -> SessionState {
    SessionState::new(
        id,
        resolve_agent("plan").expect("plan agent"),
        Config {
            model: "m/g".into(),
            ..Config::default()
        },
        mock as Arc<dyn opencoder_llm::ChatStream>,
        std::env::temp_dir(),
    )
    .with_store(store)
    .mark_session_created()
}

/// Persist `msgs` and mirror them into the live transcript — the faithful
/// mid-session state the fold runs against.
async fn seed_transcript(store: &Arc<dyn Store>, id: &str, msgs: Vec<Message>) {
    for m in &msgs {
        store.append_message(id, m).await.unwrap();
    }
}

/// Drain the UI bridge until it closes (or a generous timeout). Deterministic:
/// every sender is dropped when `process_cmd` and its forwarder finish.
async fn drain(rx: &mut mpsc::Receiver<UiEvent>) -> Vec<UiEvent> {
    let mut events = Vec::new();
    let collect = async {
        while let Some(ev) = rx.recv().await {
            events.push(ev);
        }
    };
    let _ = tokio::time::timeout(Duration::from_secs(10), collect).await;
    events
}

fn reset_transcript(events: &[UiEvent]) -> Vec<Message> {
    events
        .iter()
        .find_map(|e| match e {
            UiEvent::Session(SessionEvent::TranscriptReset(msgs)) => Some(msgs.clone()),
            _ => None,
        })
        .expect("TranscriptReset must be emitted")
}

/// The Shift+Tab fold: `/clear_context` folds the transcript to ONE synthetic
/// seed (the newest assistant say travels as neutral prior context) and then
/// executes it — exactly one LLM turn, chatter dropped, agent untouched.
#[tokio::test]
async fn clear_context_folds_transcript_and_feeds_seed_to_model() {
    let store = mem_store().await;
    store
        .create_session(&SessionMeta {
            id: "clear-fold".into(),
            agent: Some("act".into()),
            ..Default::default()
        })
        .await
        .unwrap();

    let mock = Arc::new(
        MockChatClient::new().push_script(vec![text_done("continuing from the preserved say")]),
    );
    let mut sess = act_session("clear-fold", mock.clone(), store.clone());
    seed_transcript(
        &store,
        "clear-fold",
        vec![
            Message::user("u1", "investigate the failing test"),
            assistant_with_text("a1", "let me explore the codebase first..."),
            assistant_with_text("a2", "the flaky test needs a retry guard"),
        ],
    )
    .await;
    sess.messages = vec![
        Message::user("u1", "investigate the failing test"),
        assistant_with_text("a1", "let me explore the codebase first..."),
        assistant_with_text("a2", "the flaky test needs a retry guard"),
    ];

    let (tx, mut rx) = mpsc::channel::<UiEvent>(64);
    let quit = process_cmd(
        UiCmd::Prompt("/clear_context".into(), vec![]),
        &mut sess,
        &tx,
    )
    .await;
    assert!(!quit, "the fold must not signal quit");
    let events = drain(&mut rx).await;

    // (1) TranscriptReset carries exactly one synthetic seed message.
    let reset = reset_transcript(&events);
    assert_eq!(
        reset.len(),
        1,
        "reset transcript must hold one seed message"
    );
    let seed = &reset[0];
    assert!(seed.synthetic, "the seed must be a synthetic message");
    let seed_body = seed.text();
    assert!(
        seed_body.contains("[Context cleared.") && seed_body.contains("continuity context"),
        "the seed must wrap the preserved say as neutral prior context: {seed_body}"
    );
    assert!(
        seed_body.contains("the flaky test needs a retry guard"),
        "the newest assistant say is preserved verbatim, got: {seed_body}"
    );
    assert!(
        !seed_body.contains("explore the codebase first"),
        "earlier chatter must be dropped, got: {seed_body}"
    );

    // (2) The fold keeps the agent for an already-act session: no
    // AgentSwitch, agent still act.
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, UiEvent::Session(SessionEvent::AgentSwitch(_)))),
        "the fold must not switch the agent"
    );
    assert_eq!(sess.agent.name, "act", "agent stays act across the fold");

    // (3) The preserved seed is executed: one LLM turn; live transcript is
    // seed + the new assistant reply.
    assert_eq!(
        mock.call_count(),
        1,
        "the fold falls through to exactly one seeded execution turn"
    );
    assert_eq!(sess.messages.len(), 2, "seed + the execution reply");
    assert_eq!(sess.messages[0].id, seed.id, "the seed stays message #1");

    // (4) The boundary persists as the seed flavour and resume reconstructs
    // the folded transcript: seed + post-boundary reply, no cleared history.
    let meta = store
        .get_session("clear-fold")
        .await
        .unwrap()
        .expect("session row exists");
    assert!(
        meta.handoff_seq.is_some(),
        "the fold must persist a resume boundary"
    );
    let marker = meta.handoff_plan.as_deref().unwrap_or("");
    assert!(
        is_clear_context_seed(marker),
        "the persisted marker must be the seed (last-say) flavour"
    );
    assert!(
        !is_clear_context_handoff(marker),
        "a seeded fold is not the blank fresh-start sentinel"
    );
    let resumed = resume(
        store.clone(),
        "clear-fold",
        Config::default(),
        Arc::new(MockChatClient::new()) as Arc<dyn opencoder_llm::ChatStream>,
        std::env::temp_dir(),
    )
    .await
    .expect("resume succeeds");
    assert_eq!(
        resumed.messages.len(),
        2,
        "resume rebuilds seed + post-boundary reply only, got {:?}",
        resumed
            .messages
            .iter()
            .map(|m| m.text())
            .collect::<Vec<_>>()
    );
    assert!(
        resumed.messages[0].synthetic && resumed.messages[0].text().contains("continuity context"),
        "resume reconstructs the synthetic seed marker"
    );
    assert!(
        !resumed
            .messages
            .iter()
            .any(|m| m.text().contains("explore the codebase first")),
        "the cleared history never comes back on resume"
    );
}

/// A transcript with NO assistant text has nothing to preserve: the fold
/// degrades to the blank fresh-start marker and stops without an LLM turn.
#[tokio::test]
async fn clear_context_without_assistant_text_stops_without_llm_turn() {
    let store = mem_store().await;
    store
        .create_session(&SessionMeta {
            id: "clear-blank".into(),
            agent: Some("act".into()),
            ..Default::default()
        })
        .await
        .unwrap();

    let mock = Arc::new(MockChatClient::new());
    let mut sess = act_session("clear-blank", mock.clone(), store.clone());
    let unanswered = Message::user("u1", "a request that was never answered");
    store
        .append_message("clear-blank", &unanswered)
        .await
        .unwrap();
    sess.messages = vec![unanswered];

    let (tx, mut rx) = mpsc::channel::<UiEvent>(64);
    let quit = process_cmd(
        UiCmd::Prompt("/clear_context".into(), vec![]),
        &mut sess,
        &tx,
    )
    .await;
    assert!(!quit);
    let events = drain(&mut rx).await;

    let reset = reset_transcript(&events);
    assert_eq!(reset.len(), 1);
    assert!(reset[0].synthetic, "the fresh-start marker is synthetic");

    assert_eq!(
        mock.call_count(),
        0,
        "nothing preserved -> nothing to execute: no LLM turn"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, UiEvent::Session(SessionEvent::Done))),
        "the blank fold stops as Done instead of running a turn"
    );

    let meta = store
        .get_session("clear-blank")
        .await
        .unwrap()
        .expect("session row exists");
    assert!(
        is_clear_context_handoff(meta.handoff_plan.as_deref().unwrap_or("")),
        "the blank degrade persists the fresh-start sentinel marker"
    );
}

/// Compound input (`/clear_context <rest>`): the fold keeps the seed AND the
/// trailing text is recorded as a real user prompt executed alongside it.
#[tokio::test]
async fn compound_clear_context_runs_trailing_text_as_prompt() {
    let store = mem_store().await;
    store
        .create_session(&SessionMeta {
            id: "clear-compound".into(),
            agent: Some("act".into()),
            ..Default::default()
        })
        .await
        .unwrap();

    let mock = Arc::new(MockChatClient::new().push_script(vec![text_done("done")]));
    let mut sess = act_session("clear-compound", mock.clone(), store.clone());
    let say = assistant_with_text("a1", "the plan is ready to execute");
    store.append_message("clear-compound", &say).await.unwrap();
    sess.messages = vec![say];

    let (tx, mut rx) = mpsc::channel::<UiEvent>(64);
    let quit = process_cmd(
        UiCmd::Prompt("/clear_context write the summary".into(), vec![]),
        &mut sess,
        &tx,
    )
    .await;
    assert!(!quit);
    let _ = drain(&mut rx).await;

    assert_eq!(
        mock.call_count(),
        1,
        "one seeded execution turn runs the compound rest"
    );
    let user_bodies: Vec<String> = sess
        .messages
        .iter()
        .filter(|m| !m.synthetic && m.role == Role::User)
        .map(|m| m.text())
        .collect();
    assert!(
        user_bodies.iter().any(|b| b.contains("write the summary")),
        "the trailing text must be recorded as a real user prompt, got {user_bodies:?}"
    );
    assert!(
        sess.messages
            .iter()
            .any(|m| m.synthetic && m.text().contains("the plan is ready to execute")),
        "the preserved say still travels as the seed"
    );
}

/// `/act` after the fold is a pure state switch: it persists, emits
/// AgentSwitch, and never folds or re-folds the transcript.
#[tokio::test]
async fn act_switch_after_fold_is_pure_state_change() {
    let store = mem_store().await;
    store
        .create_session(&SessionMeta {
            id: "act-after-fold".into(),
            agent: Some("plan".into()),
            ..Default::default()
        })
        .await
        .unwrap();

    let mock = Arc::new(MockChatClient::new());
    let mut sess = act_session("act-after-fold", mock.clone(), store.clone());
    sess.agent = resolve_agent("plan").unwrap();
    sess.messages = vec![opencoder_session::seed_message("the preserved say")];
    let before: Vec<String> = sess.messages.iter().map(|m| m.id.clone()).collect();

    let (tx, mut rx) = mpsc::channel::<UiEvent>(64);
    let quit = process_cmd(UiCmd::Prompt("/act".into(), vec![]), &mut sess, &tx).await;
    assert!(!quit);
    let events = drain(&mut rx).await;

    assert!(
        events.iter().any(|e| matches!(
            e,
            UiEvent::Session(SessionEvent::AgentSwitch(ref n)) if n == "act"
        )),
        "AgentSwitch(act) must be emitted for the chip"
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, UiEvent::Session(SessionEvent::TranscriptReset(_)))),
        "a pure switch never folds the transcript"
    );
    let after: Vec<String> = sess.messages.iter().map(|m| m.id.clone()).collect();
    assert_eq!(after, before, "transcript untouched by the switch");
    assert_eq!(sess.agent.name, "act");
    assert_eq!(mock.call_count(), 0, "a pure switch consumes no LLM turn");

    let meta = store
        .get_session("act-after-fold")
        .await
        .unwrap()
        .expect("session row exists");
    assert_eq!(
        meta.agent.as_deref(),
        Some("act"),
        "the switch persists to the store"
    );
}

/// Plan fold: preserve the plan as a directive, switch to act after the reset,
/// execute exactly one LLM turn, and persist the converged agent.
#[tokio::test]
async fn plan_clear_context_hands_off_and_executes_under_act() {
    let store = mem_store().await;
    store
        .create_session(&SessionMeta {
            id: "plan-fold".into(),
            agent: Some("plan".into()),
            ..Default::default()
        })
        .await
        .unwrap();

    let mock = Arc::new(
        MockChatClient::new().push_script(vec![text_done("continuing from the preserved say")]),
    );
    let mut sess = plan_session("plan-fold", mock.clone(), store.clone());
    let say = assistant_with_text("a1", "the plan answer to keep");
    seed_transcript(&store, "plan-fold", vec![say.clone()]).await;
    sess.messages = vec![say];

    let (tx, mut rx) = mpsc::channel::<UiEvent>(64);
    let quit = process_cmd(
        UiCmd::Prompt("/clear_context".into(), vec![]),
        &mut sess,
        &tx,
    )
    .await;
    assert!(!quit, "the fold must not signal quit");
    let events = drain(&mut rx).await;

    let reset_idx = events
        .iter()
        .position(|e| matches!(e, UiEvent::Session(SessionEvent::TranscriptReset(_))))
        .expect("TranscriptReset must be emitted");
    let switch_idx = events
        .iter()
        .position(
            |e| matches!(e, UiEvent::Session(SessionEvent::AgentSwitch(name)) if name == "act"),
        )
        .expect("AgentSwitch(act) must be emitted");
    assert!(reset_idx < switch_idx, "reset precedes switch: {events:?}");
    let reset = reset_transcript(&events);
    assert!(reset[0].text().contains("Execute it now"));
    assert!(reset[0].text().contains("the plan answer to keep"));
    assert_eq!(
        mock.call_count(),
        1,
        "the plan directive executes in exactly one LLM turn"
    );
    assert_eq!(sess.agent.name, "act");

    let meta = store
        .get_session("plan-fold")
        .await
        .unwrap()
        .expect("session row exists");
    assert_eq!(
        meta.agent.as_deref(),
        Some("act"),
        "the converged agent persists to the store"
    );
    assert_eq!(
        meta.handoff_plan.as_deref(),
        Some("the plan answer to keep")
    );
}
