//! Integration tests for compound control commands: `/plan review` and
//! `/plan $review` (mode switch + trailing argument / skill token),
//! submitted as the idle prompt or queued.
//!
//! Contracts:
//! - idle/queue compound arg: switch mode, then run the trailing text as a
//!   real prompt (one LLM turn, no raw command leak)
//! - `$review` token: activates the skill body and is stripped from the
//!   recorded prompt; a pure-skill compound injects the skill trigger
//!   instead of an empty user message

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config, Role};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient, Usage};
use opencoder_session::{run, SessionEvent, SessionState};
use opencoder_store::{Delivery, LibsqlStore, SessionInput, Store};

/// True when any user message of the captured request carries a skill
/// artifact — the persistent `[skill loaded]` full-body injection or the
/// transient `[active skill]` tail reminder. Under one-shot `$skill`
/// semantics (see `skill_one_shot.rs`) this is THE activation proof: the
/// skill lives exactly for the run that consumed the token.
fn request_carries_skill(req: &opencoder_llm::ChatRequest) -> bool {
    req.messages
        .iter()
        .filter(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
        .filter_map(|m| m.get("content").and_then(|c| c.as_str()))
        .any(|t| t.contains("[skill loaded]") || t.contains("[active skill]"))
}

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

/// `/plan review` submitted as the idle prompt switches to plan mode AND runs
/// "review" as a real prompt in that mode (one LLM turn), rather than leaking
/// the whole string to the model as literal text.
#[tokio::test]
async fn idle_compound_plan_arg_switches_then_runs() {
    let store = mem_store().await;
    seed(&store, "compound-idle", "act").await;
    let mock = Arc::new(MockChatClient::new().push_script(vec![done_turn("plan reply")]))
        as Arc<dyn ChatStream>;
    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "compound-idle",
        resolve_agent("act").unwrap(),
        config(),
        mock,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();

    run(&mut session, "/plan review".into(), |_| {})
        .await
        .unwrap();

    assert_eq!(session.agent.name, "plan", "switched to plan");
    // "review" was recorded as a real user prompt (not the raw "/plan review").
    let has_review = session
        .messages
        .iter()
        .any(|m| m.role == Role::User && m.text().contains("review") && !m.synthetic);
    assert!(has_review, "trailing arg recorded as a real user prompt");
    // Exactly one LLM turn ran (the "review" prompt).
    let assistant_turns = session
        .messages
        .iter()
        .filter(|m| m.role == Role::Assistant)
        .count();
    assert_eq!(assistant_turns, 1, "one assistant turn for the prompt");
}

/// `/plan review` queued as a single item: at the idle boundary the mode
/// switches (no LLM turn) and then "review" runs as a prompt (one LLM turn).
#[tokio::test]
async fn queue_compound_plan_arg_switches_then_runs() {
    let store = mem_store().await;
    seed(&store, "compound-queue", "act").await;
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![done_turn("kickoff")])
            .push_script(vec![done_turn("review reply")]),
    ) as Arc<dyn ChatStream>;
    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "compound-queue",
        resolve_agent("act").unwrap(),
        config(),
        mock,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();

    store
        .admit_input(&mk_input("compound-queue", Delivery::Queue, "/plan review"))
        .await
        .unwrap();

    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev_clone = events.clone();
    run(&mut session, "kickoff".into(), move |ev| {
        ev_clone.lock().unwrap().push(ev)
    })
    .await
    .unwrap();

    // Query store before locking events (avoid await-holding-lock).
    let pending = store
        .pending_inputs("compound-queue", Delivery::Queue)
        .await
        .unwrap();
    assert!(pending.is_empty(), "queue drained");

    let evs = events.lock().unwrap();
    assert!(
        evs.iter()
            .any(|e| matches!(e, SessionEvent::AgentSwitch(a) if a == "plan")),
        "AgentSwitch(plan) emitted"
    );
    assert!(
        evs.iter()
            .any(|e| matches!(e, SessionEvent::QueueConsumed { .. })),
        "the compound queue item was consumed"
    );

    let assistant_turns = session
        .messages
        .iter()
        .filter(|m| m.role == Role::Assistant)
        .count();
    assert_eq!(assistant_turns, 2, "kickoff turn + review turn");
    let has_review = session
        .messages
        .iter()
        .any(|m| m.role == Role::User && m.text().contains("review"));
    assert!(has_review, "'review' entered the transcript");
    assert_eq!(session.agent.name, "plan", "ended in plan mode");
}

/// `/plan $review` with a discoverable review skill: switches to plan AND
/// activates the skill for the run (proven by the LLM request carrying the
/// skill body), while the `$review` token is stripped from the recorded
/// prompt. One-shot: the skill is cleared when the run ends.
#[tokio::test]
async fn compound_plan_with_dollar_activates_skill() {
    let home = tempfile::tempdir().unwrap();
    let _guard = lock_home(home.path());
    // Seed built-in skills (creates ~/.opencoder/skills under the temp HOME).
    opencoder_core::seed_builtin_skills();

    let store = mem_store().await;
    seed(&store, "compound-skill", "act").await;
    let mock = Arc::new(MockChatClient::new().push_script(vec![done_turn("ok")]));
    let client: Arc<dyn ChatStream> = mock.clone();
    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "compound-skill",
        resolve_agent("act").unwrap(),
        config(),
        client,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();

    run(&mut session, "/plan $review explain it".into(), |_| {})
        .await
        .unwrap();

    assert_eq!(session.agent.name, "plan", "switched to plan");
    // One-shot semantics: activation is proven by the run's own LLM request
    // carrying the skill body/reminder, and the skill is cleared once the
    // run ends (see tests/skill_one_shot.rs for the full contract).
    let requests = mock.requests();
    assert!(
        requests.len() == 1 && request_carries_skill(&requests[0]),
        "the run's LLM request must carry the activated review skill"
    );
    assert!(
        session.skill_prompt_cloned().is_none(),
        "one-shot: skill cleared after the run ends"
    );
    // The recorded prompt has the `$review` token stripped, keeps the text.
    let has_explain = session
        .messages
        .iter()
        .any(|m| m.role == Role::User && m.text().contains("explain it"));
    assert!(has_explain, "remaining text recorded without the token");
    let has_dollar = session
        .messages
        .iter()
        .any(|m| m.role == Role::User && m.text().contains("$review"));
    assert!(!has_dollar, "$review token stripped from the prompt");
}

/// `/plan $review` queued with NO trailing text (pure-skill compound): the
/// mode switches, the skill body activates, and instead of recording an empty
/// user message, the skill trigger ("The active skill is now in effect…") is
/// injected so the model acts on the skill body already in the system prompt.
#[tokio::test]
async fn queue_compound_pure_skill_injects_trigger() {
    let home = tempfile::tempdir().unwrap();
    let _guard = lock_home(home.path());
    opencoder_core::seed_builtin_skills();

    let store = mem_store().await;
    seed(&store, "pure-skill-queue", "act").await;
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![done_turn("kickoff")])
            .push_script(vec![done_turn("skill reply")]),
    );
    let client: Arc<dyn ChatStream> = mock.clone();
    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "pure-skill-queue",
        resolve_agent("act").unwrap(),
        config(),
        client,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();

    store
        .admit_input(&mk_input(
            "pure-skill-queue",
            Delivery::Queue,
            "/plan $review",
        ))
        .await
        .unwrap();

    run(&mut session, "kickoff".into(), |_| {}).await.unwrap();

    assert_eq!(session.agent.name, "plan", "switched to plan");
    // One-shot semantics: the drain turn's LLM request carries the skill;
    // after the run ends the skill is cleared.
    let requests = mock.requests();
    assert!(
        requests.len() == 2 && request_carries_skill(&requests[1]),
        "the drain turn's LLM request must carry the activated skill"
    );
    assert!(
        session.skill_prompt_cloned().is_none(),
        "one-shot: skill cleared after the run ends"
    );

    // The trigger message is in the transcript (not an empty string).
    let user_msgs: Vec<String> = session
        .messages
        .iter()
        .filter(|m| m.role == Role::User)
        .map(|m| m.text())
        .collect();
    let has_trigger = user_msgs
        .iter()
        .any(|t| t.contains("active skill is now in effect"));
    assert!(has_trigger, "trigger injected: {:?}", user_msgs);

    // No empty user message was recorded.
    assert!(
        !user_msgs.iter().any(|t| t.is_empty()),
        "no empty user message: {:?}",
        user_msgs
    );
}

/// `/plan $review` as the IDLE prompt (direct run, not queued/drained):
/// switches to plan, activates the skill, and injects the skill trigger so
/// the model begins executing the skill body. This is the path the TUI idle
/// submit takes when the frontend forwards the raw compound text.
#[tokio::test]
async fn idle_compound_plan_pure_skill_injects_trigger() {
    let home = tempfile::tempdir().unwrap();
    let _guard = lock_home(home.path());
    opencoder_core::seed_builtin_skills();

    let store = mem_store().await;
    seed(&store, "idle-pure-skill", "act").await;
    let mock = Arc::new(MockChatClient::new().push_script(vec![done_turn("skill reply")]));
    let client: Arc<dyn ChatStream> = mock.clone();
    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "idle-pure-skill",
        resolve_agent("act").unwrap(),
        config(),
        client,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();

    run(&mut session, "/plan $review".into(), |_| {})
        .await
        .unwrap();

    assert_eq!(session.agent.name, "plan", "switched to plan");
    // One-shot semantics: the run's own LLM request carries the skill; the
    // skill is cleared once the run ends.
    let requests = mock.requests();
    assert!(
        requests.len() == 1 && request_carries_skill(&requests[0]),
        "the LLM request must carry the activated skill"
    );
    assert!(
        session.skill_prompt_cloned().is_none(),
        "one-shot: skill cleared after the run ends"
    );

    let user_msgs: Vec<String> = session
        .messages
        .iter()
        .filter(|m| m.role == Role::User)
        .map(|m| m.text())
        .collect();
    let has_trigger = user_msgs
        .iter()
        .any(|t| t.contains("active skill is now in effect"));
    assert!(has_trigger, "trigger injected: {:?}", user_msgs);

    assert!(
        !user_msgs.iter().any(|t| t.is_empty()),
        "no empty user message: {:?}",
        user_msgs
    );

    let assistant_turns = session
        .messages
        .iter()
        .filter(|m| m.role == Role::Assistant)
        .count();
    assert_eq!(
        assistant_turns, 1,
        "one assistant turn for the skill trigger"
    );
}
