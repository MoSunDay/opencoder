//! Integration tests for plain skill prompts: `$review do the work`
//! submitted queued or steered, with NO `/plan` prefix.
//!
//! Contracts:
//! - the `$review` token activates the skill body and is stripped from the
//!   recorded prompt; the remaining text runs as a real prompt and the agent
//!   stays unchanged (no mode switch)

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config, Role};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient, Usage};
use opencoder_session::{run, SessionState};
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

// ---------------------------------------------------------------------------
// Compound control commands: `/plan review` and `/act review` (mode switch +
// trailing argument run as a prompt in the new mode).
// ---------------------------------------------------------------------------

/// Serializes tests that mutate process-global HOME. `&HOME_MUTEX` is
/// `&'static`, so the guard is `MutexGuard<'static>` without lifetime tricks.
static HOME_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// RAII: points HOME + XDG_CONFIG_HOME at `home` (serialized via HOME_MUTEX).
struct HomeGuard {
    prev_home: Option<std::ffi::OsString>,
    prev_xdg: Option<std::ffi::OsString>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

fn lock_home(home: &std::path::Path) -> HomeGuard {
    let _lock = HOME_MUTEX.lock().unwrap();
    let prev_home = std::env::var_os("HOME");
    let prev_xdg = std::env::var_os("XDG_CONFIG_HOME");
    std::env::set_var("HOME", home);
    std::env::set_var("XDG_CONFIG_HOME", home);
    HomeGuard {
        prev_home,
        prev_xdg,
        _lock,
    }
}

impl Drop for HomeGuard {
    fn drop(&mut self) {
        match self.prev_home.take() {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
        match self.prev_xdg.take() {
            Some(h) => std::env::set_var("XDG_CONFIG_HOME", h),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
    }
}

/// Plain `$review do the work` queued with NO `/plan` prefix: the skill body
/// activates and the `$review` token is stripped, leaving "do the work" as the
/// recorded prompt. Agent stays "act" (no switch).
#[tokio::test]
async fn queue_plain_skill_prompt_resolves() {
    let home = tempfile::tempdir().unwrap();
    let _guard = lock_home(home.path());
    opencoder_core::seed_builtin_skills();

    let store = mem_store().await;
    seed(&store, "plain-skill-queue", "act").await;
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![done_turn("kickoff")])
            .push_script(vec![done_turn("work reply")]),
    ) as Arc<dyn ChatStream>;
    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "plain-skill-queue",
        resolve_agent("act").unwrap(),
        config(),
        mock,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();

    store
        .admit_input(&mk_input(
            "plain-skill-queue",
            Delivery::Queue,
            "$review do the work",
        ))
        .await
        .unwrap();

    run(&mut session, "kickoff".into(), |_| {}).await.unwrap();

    assert_eq!(
        session.agent.name, "act",
        "no agent switch for plain prompt"
    );
    // One-shot: the skill was active DURING the run (its body was injected
    // into the transcript) and is cleared once the run ends.
    assert!(
        session
            .messages
            .iter()
            .any(|m| m.text().starts_with("[skill loaded] ")),
        "skill was active during the run (body injected)"
    );
    assert!(
        session.skill_prompt_cloned().is_none(),
        "one-shot: skill cleared at run end"
    );

    let user_msgs: Vec<String> = session
        .messages
        .iter()
        .filter(|m| m.role == Role::User)
        .map(|m| m.text())
        .collect();
    assert!(
        user_msgs.iter().any(|t| t.contains("do the work")),
        "remaining text recorded: {:?}",
        user_msgs
    );
    assert!(
        !user_msgs.iter().any(|t| t.contains("$review")),
        "$review token stripped: {:?}",
        user_msgs
    );
}

/// Plain `$review analyze this` steered during a turn: the skill body
/// activates and the `$review` token is stripped from the recorded steer text.
#[tokio::test]
async fn steer_plain_skill_prompt_resolves() {
    let home = tempfile::tempdir().unwrap();
    let _guard = lock_home(home.path());
    opencoder_core::seed_builtin_skills();

    let store = mem_store().await;
    seed(&store, "plain-skill-steer", "act").await;
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![done_turn("kickoff")])
            .push_script(vec![done_turn("steered reply")]),
    ) as Arc<dyn ChatStream>;
    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "plain-skill-steer",
        resolve_agent("act").unwrap(),
        config(),
        mock,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();

    store
        .admit_input(&mk_input(
            "plain-skill-steer",
            Delivery::Steer,
            "$review analyze this",
        ))
        .await
        .unwrap();

    run(&mut session, "kickoff".into(), |_| {}).await.unwrap();

    // One-shot: activation is proven by the in-run body injection; the run's
    // end clears the skill again.
    assert!(
        session
            .messages
            .iter()
            .any(|m| m.text().starts_with("[skill loaded] ")),
        "skill was active during the run (body injected)"
    );
    assert!(
        session.skill_prompt_cloned().is_none(),
        "one-shot: skill cleared at run end"
    );

    let user_msgs: Vec<String> = session
        .messages
        .iter()
        .filter(|m| m.role == Role::User)
        .map(|m| m.text())
        .collect();
    assert!(
        user_msgs.iter().any(|t| t.contains("analyze this")),
        "remaining text recorded: {:?}",
        user_msgs
    );
    assert!(
        !user_msgs.iter().any(|t| t.contains("$review")),
        "$review token stripped: {:?}",
        user_msgs
    );
}

/// A pure `$review` queued item (token only, no text): the drain resolves the
/// skill at consumption and injects `SKILL_TRIGGER` — the deferred twin of
/// the TUI's removed "queue the trigger at submit time" branch.
#[tokio::test]
async fn queue_pure_skill_prompt_injects_trigger() {
    let home = tempfile::tempdir().unwrap();
    let _guard = lock_home(home.path());
    opencoder_core::seed_builtin_skills();

    let store = mem_store().await;
    seed(&store, "pure-skill-queue", "act").await;
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![done_turn("kickoff")])
            .push_script(vec![done_turn("skill reply")]),
    ) as Arc<dyn ChatStream>;
    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "pure-skill-queue",
        resolve_agent("act").unwrap(),
        config(),
        mock,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();

    store
        .admit_input(&mk_input("pure-skill-queue", Delivery::Queue, "$review"))
        .await
        .unwrap();

    run(&mut session, "kickoff".into(), |_| {}).await.unwrap();

    assert!(
        session.skill_prompt_cloned().is_none(),
        "one-shot: pure $review item activated the skill for its run, then the run end cleared it"
    );
    assert!(
        session.messages.iter().any(|m| m.role == Role::User
            && m.synthetic
            && m.text() == opencoder_session::skill_resolve::SKILL_TRIGGER),
        "SKILL_TRIGGER injected for the pure-skill queue item: {:?}",
        session
            .messages
            .iter()
            .map(|m| (m.role, m.text()))
            .collect::<Vec<_>>()
    );
    assert!(
        !session
            .messages
            .iter()
            .any(|m| m.text().contains("$review")),
        "the $review token never reaches the transcript"
    );
}

/// Consumption-time persistence: `sessions.skill` stays NULL while the item
/// sits queued (no eager write at admit) and lands only after the drain
/// resolved the token — the timing the resume path depends on.
#[tokio::test]
async fn queued_skill_persists_at_consumption_not_admit() {
    let home = tempfile::tempdir().unwrap();
    let _guard = lock_home(home.path());
    opencoder_core::seed_builtin_skills();

    let store = mem_store().await;
    seed(&store, "persist-skill-queue", "act").await;
    // The queued turn's LLM call HANGS so the test can observe the store at
    // the exact consumption moment — after the drain resolved `$review` but
    // before the one-shot run-end clear lands.
    let notify = Arc::new(tokio::sync::Notify::new());
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![done_turn("kickoff")])
            .push_hang(notify.clone()),
    );
    let client: Arc<dyn ChatStream> = mock.clone();
    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "persist-skill-queue",
        resolve_agent("act").unwrap(),
        config(),
        client,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();

    store
        .admit_input(&mk_input(
            "persist-skill-queue",
            Delivery::Queue,
            "$review do the work",
        ))
        .await
        .unwrap();

    assert!(
        store
            .get_session("persist-skill-queue")
            .await
            .unwrap()
            .and_then(|m| m.skill)
            .is_none(),
        "no skill persisted while the item is merely queued"
    );

    let run_task = tokio::spawn(async move {
        run(&mut session, "kickoff".into(), |_| {}).await.unwrap();
    });
    // Wait until the queued turn's LLM call is in flight (call #2).
    while mock.call_count() < 2 {
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }

    let midrun = store
        .get_session("persist-skill-queue")
        .await
        .unwrap()
        .and_then(|m| m.skill);
    assert!(
        midrun
            .as_deref()
            .is_some_and(|b| b.starts_with("> Source: ")),
        "consumption-time activation is persisted before the run ends: {midrun:?}"
    );

    // Release the hung turn; the run completes and the one-shot clear lands.
    notify.notify_one();
    run_task.await.unwrap();

    let after = store
        .get_session("persist-skill-queue")
        .await
        .unwrap()
        .and_then(|m| m.skill);
    assert!(
        after.is_none(),
        "one-shot: the completed run clears the persisted skill: {after:?}"
    );
}
