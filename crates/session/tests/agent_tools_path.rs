//! Bash PATH-prefix coverage for file-agent tool pools: once a session
//! runs under an agent with a tools pool, `bash -lc` invocations must be
//! able to resolve binaries out of the pinned pool version — while the
//! tool-call input the model/UI see stays the user's original text (the
//! export prefix lives only in the executed script). Config reload must
//! re-pin the snapshot so a bumped pool version goes live on the next
//! call without a restart.

mod common;

use std::sync::Arc;

use common::agent_fixtures::{
    make_executable, scoped_agent_roots, write_agent_card, write_pool_version,
};
use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::{run, SessionEvent, SessionState};
use opencoder_store::{LibsqlStore, SessionMeta, Store};

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

fn bash_turns() -> Vec<Vec<LlmEvent>> {
    vec![
        vec![LlmEvent::Completed {
            text: String::new(),
            tool_calls: vec![CompletedToolCall {
                id: "tu0".into(),
                name: "bash".into(),
                input: serde_json::json!({"command": "command -v probe-tool && probe-tool"}),
            }],
            usage: Some(Usage::default()),
        }],
        vec![LlmEvent::Completed {
            text: "done".into(),
            tool_calls: vec![],
            usage: Some(Usage::default()),
        }],
    ]
}

async fn make_session(
    store: &Arc<dyn Store>,
    id: &str,
    client: Arc<dyn ChatStream>,
    workdir: &std::path::Path,
) -> SessionState {
    store
        .create_session(&SessionMeta {
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

/// An agent whose tools pool `t` carries `probe-tool` at `version`.
fn agent_with_tool(root: &std::path::Path, version: u32) {
    // The agent needs a resolvable soul (prompts pool) or `resolve_agent`
    // cannot switch to it at all.
    write_pool_version(root, "prompts", "worker", 1, "soul.md", "SOUL-worker identity");
    write_pool_version(
        root,
        "tools",
        "t",
        version,
        "probe-tool",
        &format!("#!/bin/sh\necho PROBE-v{version}\n"),
    );
    make_executable(&root.join("tools/t").join(format!("v{version}")).join("probe-tool"));
    write_agent_card(root, "worker", Some("worker"), None, Some("t"));
}

async fn events(session: &mut SessionState, prompt: &str) -> Vec<SessionEvent> {
    let mut seen = Vec::new();
    run(session, prompt.into(), |ev| seen.push(ev))
        .await
        .unwrap();
    seen
}

#[tokio::test]
async fn bash_resolves_pooled_tool_and_input_stays_verbatim() {
    let fx = scoped_agent_roots();
    agent_with_tool(&fx.agents, 1);

    let store = mem_store().await;
    let mock = Arc::new(MockChatClient::new().push_script(bash_turns()[0].clone()).push_script(bash_turns()[1].clone()));
    let dir = tempfile::tempdir().unwrap();
    let mut session = make_session(&store, "s1", mock, dir.path()).await;

    events(&mut session, "/agent worker").await;
    assert_eq!(session.tools_path.len(), 1, "one pinned pool dir");

    let evs = events(&mut session, "probe").await;
    let output = evs
        .iter()
        .find_map(|e| match e {
            SessionEvent::ToolEnd { name, output, .. } if name == "bash" => Some(output.clone()),
            _ => None,
        })
        .expect("bash tool result");
    assert!(
        output.contains("PROBE-v1"),
        "pooled tool executable via PATH prefix: {output}"
    );

    let start = evs
        .iter()
        .find_map(|e| match e {
            SessionEvent::ToolStart { input, .. } => Some(input.clone()),
            _ => None,
        })
        .expect("ToolStart event");
    assert!(
        !start.to_string().contains("export PATH"),
        "displayed input keeps the original text: {start}"
    );
    assert!(
        start.to_string().contains("command -v probe-tool"),
        "original command verbatim: {start}"
    );
}

#[tokio::test]
async fn without_pool_behaves_as_before() {
    let fx = scoped_agent_roots();
    write_agent_card(&fx.agents, "toolless", None, None, None);

    let store = mem_store().await;
    let mock = Arc::new(MockChatClient::new().push_script(bash_turns()[0].clone()).push_script(bash_turns()[1].clone()));
    let dir = tempfile::tempdir().unwrap();
    let mut session = make_session(&store, "s2", mock, dir.path()).await;

    events(&mut session, "/agent toolless").await;
    assert!(session.tools_path.is_empty(), "no pools pinned");

    let evs = events(&mut session, "probe").await;
    let output = evs
        .iter()
        .find_map(|e| match e {
            SessionEvent::ToolEnd { name, output, .. } if name == "bash" => Some(output.clone()),
            _ => None,
        })
        .expect("bash tool result");
    assert!(
        output.contains("exit code: 1") || output.contains("command not found"),
        "probe-tool NOT on PATH without a pool: {output}"
    );
}

#[tokio::test]
async fn config_reload_repins_the_bumped_pool() {
    let fx = scoped_agent_roots();
    agent_with_tool(&fx.agents, 1);

    let store = mem_store().await;
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(bash_turns()[0].clone())
            .push_script(bash_turns()[1].clone())
            .push_script(bash_turns()[0].clone())
            .push_script(bash_turns()[1].clone()),
    );
    let dir = tempfile::tempdir().unwrap();
    let mut session = make_session(&store, "s3", mock, dir.path()).await;

    events(&mut session, "/agent worker").await;
    let v1: Vec<_> = session
        .tools_path
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    assert!(v1.iter().any(|p| p.ends_with("v1")), "pinned v1: {v1:?}");

    // Bump the pool on disk, then reload config — snapshot must re-pin.
    write_pool_version(
        &fx.agents,
        "tools",
        "t",
        2,
        "probe-tool",
        "#!/bin/sh\necho PROBE-v2\n",
    );
    make_executable(&fx.agents.join("tools/t/v2").join("probe-tool"));
    session.apply_config_reload_keep_client(Config {
        model: "m/g".into(),
        ..Config::default()
    });

    let v2: Vec<_> = session
        .tools_path
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    assert!(
        v2.iter().any(|p| p.ends_with("v2")) && !v2.iter().any(|p| p.ends_with("v1")),
        "re-pinned to v2: {v2:?}"
    );

    let evs = events(&mut session, "probe").await;
    let output = evs
        .iter()
        .find_map(|e| match e {
            SessionEvent::ToolEnd { name, output, .. } if name == "bash" => Some(output.clone()),
            _ => None,
        })
        .expect("bash tool result");
    assert!(
        output.contains("PROBE-v2"),
        "next call resolves the bumped version: {output}"
    );
}
