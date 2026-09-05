//! Agent-scoped skill pools must shadow same-named global skills at the
//! live-session discovery choke point (`skill_resolve::resolve_inline_skills`)
//! while global-only skills keep resolving after the switch.

mod common;

use std::sync::Arc;

use common::agent_fixtures::{
    scoped_agent_roots, write_agent_card, write_global_skill, write_pool_version,
};
use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient, Usage};
use opencoder_session::{run, SessionEvent, SessionState};
use opencoder_store::{LibsqlStore, SessionMeta, Store};

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

async fn events(session: &mut SessionState, prompt: &str) -> Vec<SessionEvent> {
    let mut seen = Vec::new();
    run(session, prompt.into(), |ev| seen.push(ev))
        .await
        .unwrap();
    seen
}

#[tokio::test]
async fn agent_skill_shadows_global_and_global_only_still_resolves() {
    let fx = scoped_agent_roots();
    // Global `alpha` and `gamma` under $HOME; agent pool `alpha-set`
    // carries a DIFFERENT `alpha` body that must win after the switch.
    write_global_skill(&fx.home, "alpha", "GLOBAL-ALPHA body");
    write_global_skill(&fx.home, "gamma", "GLOBAL-GAMMA body");
    write_pool_version(
        &fx.agents,
        "skills",
        "alpha-set",
        1,
        "alpha/SKILL.md",
        "AGENT-ALPHA body",
    );
    // The agent needs a resolvable soul (prompts pool) or `resolve_agent`
    // cannot switch to it at all.
    write_pool_version(
        &fx.agents,
        "prompts",
        "worker",
        1,
        "soul.md",
        "SOUL-worker identity",
    );
    write_agent_card(
        &fx.agents,
        "worker",
        Some("worker"),
        Some("alpha-set"),
        None,
    );

    let store = mem_store().await;
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(done_turn())
            .push_script(done_turn())
            .push_script(done_turn()),
    );
    let dir = tempfile::tempdir().unwrap();
    let mut session = make_session(&store, "s1", mock, dir.path()).await;

    // Baseline: before the switch, the global alpha body is discovered.
    // Resolved bodies land on the session's skill prompt, not in the
    // returned (token-stripped) text.
    let before =
        opencoder_session::skill_resolve::resolve_inline_skills(&session, "use $alpha and $gamma");
    assert!(
        !before.contains("$alpha") && !before.contains("$gamma") && before.contains("and"),
        "tokens stripped, rest verbatim: {before}"
    );
    let prompt_before = session.skill_prompt_cloned().expect("skills activated");
    assert!(
        prompt_before.contains("GLOBAL-ALPHA"),
        "pre-switch discovery sees the global alpha: {prompt_before}"
    );
    assert!(
        prompt_before.contains("GLOBAL-GAMMA"),
        "global-only skill always resolves: {prompt_before}"
    );

    events(&mut session, "/agent worker").await;
    assert_eq!(
        session.agent.name, "worker",
        "file agent resolved and applied"
    );
    assert!(
        session
            .skill_roots
            .iter()
            .any(|p| p.to_string_lossy().contains("alpha-set")),
        "agent skill root pinned: {:?}",
        session.skill_roots
    );

    opencoder_session::skill_resolve::resolve_inline_skills(&session, "use $alpha and $gamma");
    let prompt_after = session.skill_prompt_cloned().expect("skills activated");
    assert!(
        prompt_after.contains("AGENT-ALPHA"),
        "post-switch discovery prefers the agent-pool alpha: {prompt_after}"
    );
    assert!(
        !prompt_after.contains("GLOBAL-ALPHA"),
        "shadowed global body must not leak: {prompt_after}"
    );
    assert!(
        prompt_after.contains("GLOBAL-GAMMA"),
        "global-only skill still resolves after the switch: {prompt_after}"
    );
}

#[tokio::test]
async fn switch_back_restores_global_only_discovery() {
    let fx = scoped_agent_roots();
    write_global_skill(&fx.home, "alpha", "GLOBAL-ALPHA body");
    write_pool_version(
        &fx.agents,
        "skills",
        "alpha-set",
        1,
        "alpha/SKILL.md",
        "AGENT-ALPHA body",
    );
    write_pool_version(
        &fx.agents,
        "prompts",
        "worker",
        1,
        "soul.md",
        "SOUL-worker identity",
    );
    write_agent_card(
        &fx.agents,
        "worker",
        Some("worker"),
        Some("alpha-set"),
        None,
    );

    let store = mem_store().await;
    let mock = Arc::new(MockChatClient::new().push_script(done_turn()));
    let dir = tempfile::tempdir().unwrap();
    let mut session = make_session(&store, "s2", mock, dir.path()).await;

    events(&mut session, "/agent worker").await;
    events(&mut session, "/agent act").await;
    assert!(
        session
            .skill_roots
            .iter()
            .all(|p| !p.to_string_lossy().contains("alpha-set")),
        "agent roots dropped on builtin switch: {:?}",
        session.skill_roots
    );
    opencoder_session::skill_resolve::resolve_inline_skills(&session, "use $alpha");
    let back = session.skill_prompt_cloned().expect("skills activated");
    assert!(
        back.contains("GLOBAL-ALPHA") && !back.contains("AGENT-ALPHA"),
        "global body restored: {back}"
    );
}
