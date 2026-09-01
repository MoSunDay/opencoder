//! `/plan` and `/act` are PURE state switches driven through the worker:
//! they persist the new agent (so `resume()` and the /task picker see it),
//! emit `AgentSwitch` for the chip, and never fold the transcript or record a
//! user message.

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config, ContentBlock, Message};
use opencoder_llm::MockChatClient;
use opencoder_session::{resume, SessionEvent, SessionState};
use opencoder_store::{LibsqlStore, SessionMeta, Store};
use opencoder_tui::worker::{process_cmd, UiCmd, UiEvent};
use tokio::sync::mpsc;

fn assistant_with_text(id: &str, text: &str) -> Message {
    let mut m = Message::assistant(id);
    m.blocks.push(ContentBlock::text(text));
    m
}

async fn running_session(id: &str, agent: &str) -> (SessionState, Arc<MockChatClient>, Arc<dyn Store>) {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    store
        .create_session(&SessionMeta {
            id: id.into(),
            agent: Some(agent.into()),
            ..Default::default()
        })
        .await
        .unwrap();
    let say = assistant_with_text("a1", "work in progress");
    store.append_message(id, &say).await.unwrap();

    let mock = Arc::new(MockChatClient::new());
    let mut sess = SessionState::new(
        id,
        resolve_agent(agent).unwrap(),
        Config {
            model: "m/g".into(),
            ..Config::default()
        },
        mock.clone() as Arc<dyn opencoder_llm::ChatStream>,
        std::env::temp_dir(),
    )
    .with_store(store.clone())
    .mark_session_created();
    sess.messages = vec![Message::user("u1", "start the work"), say];
    (sess, mock, store)
}

async fn submit(sess: &mut SessionState, cmd: &str) -> Vec<UiEvent> {
    let (tx, mut rx) = mpsc::channel::<UiEvent>(64);
    let quit = process_cmd(UiCmd::Prompt(cmd.into(), vec![]), sess, &tx).await;
    assert!(!quit, "{cmd} must not signal quit");
    let mut events = Vec::new();
    let collect = async {
        while let Some(ev) = rx.recv().await {
            events.push(ev);
        }
    };
    let _ = tokio::time::timeout(std::time::Duration::from_secs(10), collect).await;
    events
}

fn transcript_fingerprint(sess: &SessionState) -> Vec<String> {
    sess.messages
        .iter()
        .map(|m| format!("{}:{:?}:{}", m.id, m.role, m.text()))
        .collect()
}

/// `/plan` persists the switch and resume() restores the plan agent —
/// the same session comes back plan after a process restart.
#[tokio::test]
async fn plan_switch_persists_and_survives_resume() {
    let (mut sess, mock, store) = running_session("switch-plan", "act").await;
    let before = transcript_fingerprint(&sess);

    let events = submit(&mut sess, "/plan").await;
    assert!(events.iter().any(|e| matches!(
        e,
        UiEvent::Session(SessionEvent::AgentSwitch(ref n)) if n == "plan"
    )));
    assert_eq!(sess.agent.name, "plan");
    assert_eq!(
        mock.call_count(),
        0,
        "a pure switch consumes no LLM turn"
    );

    let meta = store.get_session("switch-plan").await.unwrap().unwrap();
    assert_eq!(
        meta.agent.as_deref(),
        Some("plan"),
        "the switch must persist to the store"
    );

    let resumed = resume(
        store.clone(),
        "switch-plan",
        Config::default(),
        Arc::new(MockChatClient::new()) as Arc<dyn opencoder_llm::ChatStream>,
        std::env::temp_dir(),
    )
    .await
    .expect("resume succeeds");
    assert_eq!(
        resumed.agent.name, "plan",
        "resume must honor the persisted agent switch"
    );

    assert_eq!(
        transcript_fingerprint(&sess),
        before,
        "the switch must never touch the transcript"
    );
}

/// `/act` switches back the same way: persisted, announced, transcript-safe.
#[tokio::test]
async fn act_roundtrip_back_is_persisted() {
    let (mut sess, mock, store) = running_session("switch-roundtrip", "plan").await;
    sess.agent = resolve_agent("plan").unwrap();

    let events = submit(&mut sess, "/act").await;
    assert!(events.iter().any(|e| matches!(
        e,
        UiEvent::Session(SessionEvent::AgentSwitch(ref n)) if n == "act"
    )));
    assert_eq!(sess.agent.name, "act");
    assert_eq!(mock.call_count(), 0);

    let meta = store
        .get_session("switch-roundtrip")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(meta.agent.as_deref(), Some("act"));

    let resumed = resume(
        store,
        "switch-roundtrip",
        Config::default(),
        Arc::new(MockChatClient::new()) as Arc<dyn opencoder_llm::ChatStream>,
        std::env::temp_dir(),
    )
    .await
    .unwrap();
    assert_eq!(resumed.agent.name, "act");
}

/// A switch is NOT a clear: no TranscriptReset event, no user message
/// recorded, no handoff boundary written.
#[tokio::test]
async fn switch_never_folds_or_records() {
    let (mut sess, mock, store) = running_session("switch-no-fold", "act").await;
    let before = transcript_fingerprint(&sess);

    let events = submit(&mut sess, "/plan").await;
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, UiEvent::Session(SessionEvent::TranscriptReset(_)))),
        "a switch must not emit TranscriptReset"
    );

    let meta = store.get_session("switch-no-fold").await.unwrap().unwrap();
    assert!(
        meta.handoff_seq.is_none(),
        "a switch must not write a handoff boundary"
    );
    assert_eq!(
        transcript_fingerprint(&sess),
        before,
        "the switch must not fold or append anything"
    );

    // And a follow-up real prompt on the switched agent still executes (the
    // switch left the runtime fully usable).
    mock.queue_script(vec![opencoder_llm::LlmEvent::Completed {
        text: "ack".into(),
        tool_calls: vec![],
        usage: None,
    }]);
    let (tx, mut rx) = mpsc::channel::<UiEvent>(64);
    let quit = process_cmd(
        UiCmd::Prompt("continue in plan".into(), vec![]),
        &mut sess,
        &tx,
    )
    .await;
    assert!(!quit);
    while rx.try_recv().is_ok() {}
    assert_eq!(mock.call_count(), 1, "the follow-up prompt runs one turn");
    assert!(
        sess.messages
            .iter()
            .any(|m| !m.synthetic && m.text().contains("continue in plan")),
        "the follow-up prompt is recorded as a real user message"
    );
}
