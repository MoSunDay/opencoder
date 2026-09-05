//! End-to-end coverage for the `/agent <name>` control command against
//! real file-based agents: the switch must be a single idempotent
//! transaction (event + persisted `meta.agent` + refreshed pool
//! snapshots), unresolvable names must fail soft without touching state,
//! builtins keep working, `/agent` alone lists the pool, and a compound
//! `/agent <name> <prompt>` records the remainder as the next turn's
//! user input executed under the new agent.

mod common;

use std::sync::Arc;

use common::agent_fixtures::{scoped_agent_roots, write_agent_card, write_full_agent};
use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient, Usage};
use opencoder_session::{run, SessionEvent, SessionState};
use opencoder_store::{LibsqlStore, Store};

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

fn done_turn() -> Vec<LlmEvent> {
    vec![LlmEvent::Completed {
        text: "ok".into(),
        tool_calls: vec![],
        usage: Some(Usage::default()),
    }]
}

async fn make_session(
    store: &Arc<dyn Store>,
    id: &str,
    client: Arc<dyn ChatStream>,
    workdir: &std::path::Path,
) -> SessionState {
    store
        .create_session(&opencoder_store::SessionMeta {
            id: id.into(),
            agent: Some("act".into()),
            model: Some("m/g".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    SessionState::new(
        id,
        resolve_agent("act").unwrap(),
        Config {
            model: "m/g".into(),
            ..Config::default()
        },
        client,
        workdir.to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created()
}

async fn events(session: &mut SessionState, prompt: &str) -> Vec<SessionEvent> {
    let mut seen = Vec::new();
    run(session, prompt.into(), |ev| seen.push(ev))
        .await
        .unwrap();
    seen
}

#[tokio::test]
async fn switch_to_file_agent_switches_soul_and_pools() {
    let fx = scoped_agent_roots();
    write_full_agent(&fx.agents, "worker", true, true, 1);

    let store = mem_store().await;
    let mock = Arc::new(MockChatClient::new().push_script(done_turn()));
    let dir = tempfile::tempdir().unwrap();
    let mut session = make_session(&store, "s1", mock, dir.path()).await;

    let evs = events(&mut session, "/agent worker").await;
    assert!(
        evs.iter()
            .any(|e| matches!(e, SessionEvent::AgentSwitch(a) if a == "worker")),
        "AgentSwitch(worker) emitted, got: {evs:?}"
    );
    assert_eq!(session.agent.name, "worker");
    assert!(
        !session.tools_path.is_empty(),
        "tools pool snapshot refreshed"
    );
    assert!(
        session
            .skill_roots
            .iter()
            .any(|p| p.to_string_lossy().contains("alpha-set")),
        "skill roots include agent pools: {:?}",
        session.skill_roots
    );
    assert_eq!(
        store
            .get_session("s1")
            .await
            .unwrap()
            .unwrap()
            .agent
            .as_deref(),
        Some("worker"),
        "agent persisted"
    );

    // Builtin agents remain addressable and the pools flip back.
    events(&mut session, "/agent plan").await;
    assert_eq!(session.agent.name, "plan");
    assert!(
        session.tools_path.is_empty(),
        "builtin agent drops file tools"
    );
}

#[tokio::test]
async fn next_turn_system_prompt_carries_agent_soul() {
    let fx = scoped_agent_roots();
    write_full_agent(&fx.agents, "worker", false, false, 1);

    let store = mem_store().await;
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(done_turn())
            .push_script(done_turn()),
    );
    let dir = tempfile::tempdir().unwrap();
    let mut session = make_session(&store, "s2", mock.clone(), dir.path()).await;

    events(&mut session, "/agent worker").await;
    events(&mut session, "hello").await;

    let req = &mock.requests()[0];
    let system = req.messages[0].get("content").unwrap().to_string();
    assert!(
        system.contains("SOUL-worker"),
        "system prompt carries the file agent soul: {system}"
    );
}

#[tokio::test]
async fn unknown_agent_fails_soft_and_keeps_state() {
    let fx = scoped_agent_roots();
    write_full_agent(&fx.agents, "worker", false, false, 1);

    let store = mem_store().await;
    let mock = Arc::new(MockChatClient::new().push_script(done_turn()));
    let dir = tempfile::tempdir().unwrap();
    let mut session = make_session(&store, "s3", mock.clone(), dir.path()).await;

    events(&mut session, "/agent worker").await;
    assert_eq!(session.agent.name, "worker");

    let evs = events(&mut session, "/agent nope").await;
    let errors: Vec<&String> = evs
        .iter()
        .filter_map(|e| match e {
            SessionEvent::Error(m) => Some(m),
            _ => None,
        })
        .collect();
    assert_eq!(errors.len(), 1, "exactly one soft error, no retry: {evs:?}");
    assert!(
        errors[0].contains("unknown agent") && errors[0].contains("nope"),
        "error names the unknown agent: {}",
        errors[0]
    );
    assert_eq!(session.agent.name, "worker", "agent unchanged");
    assert_eq!(
        store
            .get_session("s3")
            .await
            .unwrap()
            .unwrap()
            .agent
            .as_deref(),
        Some("worker"),
        "store unchanged"
    );
    assert_eq!(mock.requests().len(), 0, "no LLM turn spent on the failure");
}

#[tokio::test]
async fn bare_agent_lists_pool_with_current_marker() {
    let fx = scoped_agent_roots();
    write_full_agent(&fx.agents, "worker", false, false, 1);
    write_agent_card(&fx.agents, "zeta", None, None, None);

    let store = mem_store().await;
    let mock = Arc::new(MockChatClient::new().push_script(done_turn()));
    let dir = tempfile::tempdir().unwrap();
    let mut session = make_session(&store, "s4", mock.clone(), dir.path()).await;

    let evs = events(&mut session, "/agent").await;
    let listing = evs
        .iter()
        .find_map(|e| match e {
            SessionEvent::Status(t) => Some(t.clone()),
            _ => None,
        })
        .expect("Status listing event");
    assert!(
        listing.contains("agents:"),
        "listing header present: {listing}"
    );
    assert!(listing.contains("worker"), "file agents listed: {listing}");
    assert!(
        listing.contains("zeta"),
        "all file agents listed: {listing}"
    );
    assert!(
        listing.contains("*act"),
        "current builtin marked: {listing}"
    );
    assert_eq!(mock.requests().len(), 0, "listing spends no turn");
}

#[tokio::test]
async fn compound_agent_switch_runs_rest_under_new_agent() {
    let fx = scoped_agent_roots();
    write_full_agent(&fx.agents, "worker", false, false, 1);

    let store = mem_store().await;
    let mock = Arc::new(MockChatClient::new().push_script(done_turn()));
    let dir = tempfile::tempdir().unwrap();
    let mut session = make_session(&store, "s5", mock.clone(), dir.path()).await;

    events(&mut session, "/agent worker do the thing").await;
    assert_eq!(session.agent.name, "worker", "switch applied");
    assert_eq!(
        store
            .get_session("s5")
            .await
            .unwrap()
            .unwrap()
            .agent
            .as_deref(),
        Some("worker"),
        "switch persisted before running the remainder"
    );
    assert_eq!(mock.requests().len(), 1, "remainder ran as one LLM turn");

    let req = &mock.requests()[0];
    let msgs = &req.messages;
    assert!(
        msgs.iter().any(|m| {
            m.get("role").and_then(|r| r.as_str()) == Some("user")
                && m.get("content")
                    .map(|c| c.to_string().contains("do the thing"))
                    .unwrap_or(false)
        }),
        "remainder recorded as the user turn: {msgs:?}"
    );
    let system = msgs[0].get("content").unwrap().to_string();
    assert!(
        system.contains("SOUL-worker"),
        "turn ran under the new agent's soul: {system}"
    );
}

#[tokio::test]
async fn agent_switch_same_name_is_silent_noop() {
    let fx = scoped_agent_roots();
    write_full_agent(&fx.agents, "worker", false, false, 1);

    let store = mem_store().await;
    let mock = Arc::new(MockChatClient::new().push_script(done_turn()));
    let dir = tempfile::tempdir().unwrap();
    let mut session = make_session(&store, "s6", mock.clone(), dir.path()).await;

    events(&mut session, "/agent worker").await;
    events(&mut session, "/agent worker").await;
    assert_eq!(session.agent.name, "worker");
    assert_eq!(mock.requests().len(), 0, "no-op switch spends no turn");
}
