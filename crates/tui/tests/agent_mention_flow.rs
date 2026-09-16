//! Integration tests for the `/agent` switch surface (TUI side).
//!
//! Two layers are pinned, both through the production worker path
//! (`process_cmd(UiCmd::Prompt, ...)` -- the same route the composer submit
//! takes):
//!
//! 1. Control head: `/agent <name>` rides the runner's control prefix
//!    (`opencoder_session::control_cmd::split_control_prefix` + `apply`),
//!    swaps the session's whole agent struct, emits
//!    `SessionEvent::AgentSwitch`, and persists the agent on the session
//!    row. A name that resolves to nothing is rejected with an `Error`
//!    event and leaves the session agent untouched.
//! 2. Picker key path: the real catalog (`available_primary_agents`, which
//!    mirrors the web `/api/agents` primary computation), the real
//!    keystroke handler (`handle_agent_key`) and `pick_token` produce the
//!    `/agent <name> ` composer text; submitting it applies the switch --
//!    so a picker that drops its control head (tokens silently stop
//!    switching) fails here, not just in unit tests.

use std::sync::Arc;
use tokio::sync::{Mutex, MutexGuard};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use opencoder_core::agent::set_agents_dir_override;
use opencoder_core::{AgentKind, Config};
use opencoder_llm::MockChatClient;
use opencoder_session::{SessionEvent, SessionState};
use opencoder_store::{LibsqlStore, SessionMeta, Store};
use opencoder_tui::agent_menu::{
    available_primary_agents, handle_agent_key, pick_token, AgentMenu, AgentOutcome,
};
use opencoder_tui::worker::{process_cmd, UiCmd, UiEvent};

/// Serializes tests touching the process-global agents-root override.
static OVERRIDE_LOCK: Mutex<()> = Mutex::const_new(());

/// Point the agents root at a fresh tempdir under the override lock and
/// return the dir plus the guard (must be held for the whole test body).
async fn scoped_agents() -> (tempfile::TempDir, MutexGuard<'static, ()>) {
    let dir = tempfile::tempdir().unwrap();
    let guard = OVERRIDE_LOCK.lock().await;
    set_agents_dir_override(Some(dir.path().to_path_buf()));
    (dir, guard)
}

/// Minimal resolvable file agent: a private prompt pool `prompts/<name>/v1`
/// (soul only) plus a card referencing it. The soul's first line doubles as
/// the picker description.
fn write_file_agent(root: &std::path::Path, name: &str, soul: &str) {
    let pool = root.join("prompts").join(name);
    let vdir = pool.join("v1");
    std::fs::create_dir_all(&vdir).unwrap();
    std::fs::write(vdir.join("soul.md"), soul).unwrap();
    std::fs::write(
        pool.join("meta.json"),
        format!(r#"{{ "name": "{name}", "current": 1, "history": [1] }}"#),
    )
    .unwrap();
    let adir = root.join(name);
    std::fs::create_dir_all(&adir).unwrap();
    std::fs::write(
        adir.join("meta.json"),
        format!(r#"{{ "name": "{name}", "current": {{ "prompt": "{name}" }} }}"#),
    )
    .unwrap();
}

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

/// A session whose config points at the fixture agents root.
fn session_with_dir(
    id: &str,
    agent_name: &str,
    agents_dir: &std::path::Path,
    mock: Arc<MockChatClient>,
    store: Arc<dyn Store>,
    workdir: &std::path::Path,
) -> SessionState {
    let mut cfg = Config::default();
    cfg.agent.agents_dir = Some(agents_dir.to_path_buf());
    let agent = opencoder_core::resolve_agent(agent_name).expect("agent must resolve");
    SessionState::new(id, agent, cfg, mock, workdir.to_path_buf()).with_store(store)
}

/// Collect forwarded UI events until the channel settles (the forwarder
/// task drains after the run returns), bounded by a generous timeout.
async fn drain_events(mut rx: tokio::sync::mpsc::Receiver<UiEvent>) -> Vec<SessionEvent> {
    let mut out = Vec::new();
    while let Ok(ev) = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv()).await {
        match ev {
            Some(UiEvent::Session(se)) => out.push(se),
            Some(_) => continue,
            None => break,
        }
    }
    out
}

/// `/agent <name>` switches the session's agent through the runner's
/// control head: the agent struct is swapped, an AgentSwitch event is
/// forwarded, and the session row records the new agent.
#[tokio::test]
async fn agent_control_head_switches_session_agent() {
    let (dir, _g) = scoped_agents().await;
    write_file_agent(dir.path(), "writer", "Writer soul: drafts docs.");

    let store = mem_store().await;
    store
        .create_session(&SessionMeta {
            id: "agent-switch-flow".into(),
            agent: Some("act".into()),
            ..Default::default()
        })
        .await
        .unwrap();

    let mock = Arc::new(MockChatClient::new());
    let (tx, rx) = tokio::sync::mpsc::channel::<UiEvent>(64);
    let mut sess = session_with_dir(
        "agent-switch-flow",
        "act",
        dir.path(),
        mock.clone(),
        store.clone(),
        std::path::Path::new("."),
    );

    let quit = process_cmd(
        UiCmd::Prompt("/agent writer".into(), vec![]),
        &mut sess,
        &tx,
    )
    .await;
    assert!(
        !quit,
        "a bare control command must not break the worker loop"
    );

    // The session now runs the file agent.
    assert_eq!(sess.agent.name, "writer", "session agent switched");
    assert!(sess.agent.prompt.contains("Writer soul"), "new agent body");
    assert_eq!(
        sess.agent.kind,
        AgentKind::Act,
        "file agents default to act kind"
    );

    // The switch is persisted on the session row.
    let row = store
        .get_session("agent-switch-flow")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.agent.as_deref(), Some("writer"));

    // And an AgentSwitch event reached the UI channel.
    let events = drain_events(rx).await;
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SessionEvent::AgentSwitch(n) if n == "writer")),
        "AgentSwitch(writer) must be forwarded, got {events:?}"
    );
}

/// A typo'd `/agent <name>` is rejected with an Error event and changes
/// nothing: the session agent and the persisted row stay on the old agent.
#[tokio::test]
async fn unknown_agent_name_errors_and_keeps_current_agent() {
    let (dir, _g) = scoped_agents().await;
    write_file_agent(dir.path(), "writer", "Writer soul.");

    let store = mem_store().await;
    store
        .create_session(&SessionMeta {
            id: "agent-miss-flow".into(),
            agent: Some("act".into()),
            ..Default::default()
        })
        .await
        .unwrap();

    let mock = Arc::new(MockChatClient::new());
    let (tx, rx) = tokio::sync::mpsc::channel::<UiEvent>(64);
    let mut sess = session_with_dir(
        "agent-miss-flow",
        "act",
        dir.path(),
        mock.clone(),
        store.clone(),
        std::path::Path::new("."),
    );

    let quit = process_cmd(UiCmd::Prompt("/agent nope".into(), vec![]), &mut sess, &tx).await;
    assert!(!quit, "a failed switch must not break the worker loop");

    assert_eq!(sess.agent.name, "act", "an unknown name must not switch");
    let row = store.get_session("agent-miss-flow").await.unwrap().unwrap();
    assert_eq!(
        row.agent.as_deref(),
        Some("act"),
        "persisted agent unchanged"
    );

    let events = drain_events(rx).await;
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SessionEvent::Error(m) if m.contains("nope"))),
        "the failure must name the unknown agent, got {events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, SessionEvent::AgentSwitch(_))),
        "no AgentSwitch event may fire for an unknown agent"
    );
}

/// Picker key path: `/agent` opens the picker over the real catalog, a
/// fuzzy-filtered Enter pick fills the composer with the control head, and
/// submitting it through the worker applies the switch.
#[tokio::test]
async fn picker_pick_fills_control_head_and_switches() {
    let (dir, _g) = scoped_agents().await;
    write_file_agent(dir.path(), "writer", "Writer soul: drafts docs.");

    // Catalog side: builtin primaries first, the file card appended, and
    // every row is switchable (primary only -- no `workflow` scheduler row).
    let cards = available_primary_agents();
    let names: Vec<&str> = cards.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(&names[..3], &["act", "plan", "command"], "builtin order");
    assert!(!names.contains(&"workflow"), "scheduler stays hidden");
    let writer = cards
        .iter()
        .find(|c| c.name == "writer")
        .expect("file agent must be listed");
    assert!(
        writer.description.contains("Writer soul"),
        "the card carries the soul's first line, got {:?}",
        writer.description
    );

    // Key path: filter "wr", Enter picks it, the composer gets the token.
    let mut slot = Some(AgentMenu::new(available_primary_agents()));
    for ch in "wr".chars() {
        handle_agent_key(
            &mut slot,
            KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
        );
    }
    let picked =
        match handle_agent_key(&mut slot, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)) {
            AgentOutcome::Pick(name) => name,
            other => panic!("expected a pick, got {other:?}"),
        };
    assert!(slot.is_none(), "pick closes the menu");
    assert_eq!(
        picked, "writer",
        "the fuzzy hit resolves to the writer card"
    );
    let token = pick_token(&picked);
    assert_eq!(token, "/agent writer ");

    // Submit side: the control head applies the switch for real.
    let store = mem_store().await;
    store
        .create_session(&SessionMeta {
            id: "agent-picker-flow".into(),
            agent: Some("act".into()),
            ..Default::default()
        })
        .await
        .unwrap();

    let mock = Arc::new(MockChatClient::new());
    let (tx, rx) = tokio::sync::mpsc::channel::<UiEvent>(64);
    let mut sess = session_with_dir(
        "agent-picker-flow",
        "act",
        dir.path(),
        mock.clone(),
        store.clone(),
        std::path::Path::new("."),
    );

    let quit = process_cmd(UiCmd::Prompt(token, vec![]), &mut sess, &tx).await;
    assert!(!quit, "submitting the picker token must not quit");

    assert_eq!(sess.agent.name, "writer", "the pick switched the agent");
    assert!(
        sess.agent.prompt.contains("Writer soul"),
        "the switched agent carries the file card prompt"
    );
    let row = store
        .get_session("agent-picker-flow")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.agent.as_deref(), Some("writer"), "persisted agent");
    let events = drain_events(rx).await;
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SessionEvent::AgentSwitch(n) if n == "writer")),
        "AgentSwitch(writer) must be forwarded, got {events:?}"
    );
    // No LLM round ran: a bare control command never reaches the model.
    assert!(
        mock.requests().is_empty(),
        "control head must not call the model"
    );
}
