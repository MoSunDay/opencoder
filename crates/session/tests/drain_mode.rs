//! Drain-mode tests: queued control commands and prompts consumed via the
//! pure-drain entry path — `run(session, "")` — which is how the web
//! `drain_to_completion` drives the loop.
//!
//! Before the fix, `run_loop` always called `run_one_llm_call` before
//! checking the queue, producing a "ghost" LLM turn on the stale agent mode.
//! These tests verify queued items are consumed BEFORE any LLM call.

use std::sync::{Arc, Mutex};

use opencoder_core::{resolve_agent, Config, Role};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_session::{fire_turn_cancel, run, SessionEvent, SessionState};
use opencoder_store::{Delivery, LibsqlStore, SessionInput, Store};

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

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
        usage: None,
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

fn mk_input(sid: &str, delivery: Delivery, prompt: &str) -> SessionInput {
    SessionInput {
        seq: None,
        id: opencoder_session::runner::new_id(),
        session_id: sid.into(),
        delivery,
        prompt: prompt.into(),
        images: vec![],
        display_text: None,
        admitted_seq: 0,
        promoted_seq: None,
    }
}

/// Run drain mode (`run(session, "")`), collecting events into a Vec.
async fn drain_run(session: &mut SessionState) -> Vec<SessionEvent> {
    let events = Arc::new(Mutex::new(Vec::new()));
    let ev = events.clone();
    run(session, String::new(), move |e| ev.lock().unwrap().push(e))
        .await
        .unwrap();
    Arc::try_unwrap(events)
        .map(|m| m.into_inner().unwrap())
        .unwrap_or_default()
}

fn assert_no_llm_calls(evs: &[SessionEvent]) {
    let count = evs
        .iter()
        .filter(|e| matches!(e, SessionEvent::TextDelta(_)))
        .count();
    assert_eq!(count, 0, "expected zero LLM text deltas, got {count}");
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

/// `/plan review code` queued in drain mode: the agent switches to plan
/// BEFORE any LLM call, then "review code" runs (exactly 1 LLM turn).
#[tokio::test]
async fn drain_mode_queue_plan_switches_before_llm() {
    let store = mem_store().await;
    seed(&store, "dr-plan", "act").await;

    let mock = Arc::new(MockChatClient::new().push_script(vec![done_turn("plan reply")]));
    let client: Arc<dyn ChatStream> = mock.clone();

    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "dr-plan",
        resolve_agent("act").unwrap(),
        config(),
        client,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();

    store
        .admit_input(&mk_input(
            "dr-plan",
            Delivery::Queue,
            "/plan review code",
        ))
        .await
        .unwrap();

    let evs = drain_run(&mut session).await;

    assert_eq!(
        session.agent.name, "plan",
        "switched to plan before LLM"
    );
    assert!(
        evs.iter()
            .any(|e| matches!(e, SessionEvent::AgentSwitch(a) if a == "plan")),
        "AgentSwitch(plan) emitted"
    );
    assert!(
        evs.iter().any(|e| matches!(e, SessionEvent::Done)),
        "Done emitted"
    );
    assert_eq!(
        mock.requests().len(),
        1,
        "exactly 1 LLM call (review in plan mode)"
    );
}

/// Bare `/plan` queued in drain mode: switches to plan with ZERO LLM calls.
#[tokio::test]
async fn drain_mode_queue_bare_plan_goes_idle() {
    let store = mem_store().await;
    seed(&store, "dr-bare-plan", "act").await;

    let mock = Arc::new(MockChatClient::new());
    let client: Arc<dyn ChatStream> = mock.clone();

    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "dr-bare-plan",
        resolve_agent("act").unwrap(),
        config(),
        client,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();

    store
        .admit_input(&mk_input("dr-bare-plan", Delivery::Queue, "/plan"))
        .await
        .unwrap();

    let evs = drain_run(&mut session).await;

    assert_eq!(session.agent.name, "plan", "switched to plan");
    assert!(
        evs.iter()
            .any(|e| matches!(e, SessionEvent::AgentSwitch(a) if a == "plan")),
        "AgentSwitch(plan) emitted"
    );
    assert!(
        evs.iter().any(|e| matches!(e, SessionEvent::Done)),
        "Done emitted"
    );
    assert_no_llm_calls(&evs);
    assert!(mock.requests().is_empty(), "zero LLM calls");
}

/// `/act` queued in a plan-mode session in drain mode: switches to act with
/// ZERO LLM calls.
#[tokio::test]
async fn drain_mode_queue_act_switches_before_llm() {
    let store = mem_store().await;
    seed(&store, "dr-act", "plan").await;

    let mock = Arc::new(MockChatClient::new());
    let client: Arc<dyn ChatStream> = mock.clone();

    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "dr-act",
        resolve_agent("plan").unwrap(),
        config(),
        client,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();

    store
        .admit_input(&mk_input("dr-act", Delivery::Queue, "/act"))
        .await
        .unwrap();

    let evs = drain_run(&mut session).await;

    assert_eq!(session.agent.name, "act", "switched to act");
    assert!(
        evs.iter()
            .any(|e| matches!(e, SessionEvent::AgentSwitch(a) if a == "act")),
        "AgentSwitch(act) emitted"
    );
    assert!(
        evs.iter().any(|e| matches!(e, SessionEvent::Done)),
        "Done emitted"
    );
    assert_no_llm_calls(&evs);
    assert!(mock.requests().is_empty(), "zero LLM calls");
}

/// Empty queue in drain mode: Done immediately, ZERO LLM calls (no ghost
/// turn on the stale agent).
#[tokio::test]
async fn drain_mode_empty_queue_goes_idle() {
    let store = mem_store().await;
    seed(&store, "dr-empty", "act").await;

    let mock = Arc::new(MockChatClient::new());
    let client: Arc<dyn ChatStream> = mock.clone();

    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "dr-empty",
        resolve_agent("act").unwrap(),
        config(),
        client,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();

    // No queue items admitted.
    let evs = drain_run(&mut session).await;

    assert!(
        evs.iter().any(|e| matches!(e, SessionEvent::Done)),
        "Done emitted immediately"
    );
    assert_no_llm_calls(&evs);
    assert!(mock.requests().is_empty(), "zero LLM calls — no ghost turn");
}

/// `$review analyze code` queued in drain mode: skill activates and the
/// prompt runs (exactly 1 LLM turn).
#[tokio::test]
async fn drain_mode_queue_skill_consumed() {
    let home = tempfile::tempdir().unwrap();
    let _guard = lock_home(home.path());
    opencoder_core::seed_builtin_skills();

    let store = mem_store().await;
    seed(&store, "dr-skill", "act").await;

    let mock = Arc::new(MockChatClient::new().push_script(vec![done_turn("review done")]));
    let client: Arc<dyn ChatStream> = mock.clone();

    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "dr-skill",
        resolve_agent("act").unwrap(),
        config(),
        client,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();

    store
        .admit_input(&mk_input(
            "dr-skill",
            Delivery::Queue,
            "$review analyze code",
        ))
        .await
        .unwrap();

    let evs = drain_run(&mut session).await;

    // One-shot `$skill` semantics (see skill_one_shot.rs): the skill lives
    // exactly for the run that consumed the token. Activation is proven by
    // the captured LLM request carrying a skill artifact ([skill loaded]
    // body injection or [active skill] tail reminder); the run-end hook
    // clears the skill afterwards.
    assert_eq!(
        mock.requests().len(),
        1,
        "exactly 1 LLM call (skill prompt)"
    );
    assert!(
        mock.requests().iter().any(|req| req
            .messages
            .iter()
            .filter(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
            .filter_map(|m| m.get("content").and_then(|c| c.as_str()))
            .any(|t| t.contains("[skill loaded]") || t.contains("[active skill]"))),
        "skill activated by $review token (request carries skill artifact)"
    );
    assert!(
        session.skill_prompt_cloned().is_none(),
        "one-shot skill cleared after run end"
    );
    assert!(
        evs.iter().any(|e| matches!(e, SessionEvent::Done)),
        "Done emitted"
    );
    let user_msgs: Vec<String> = session
        .messages
        .iter()
        .filter(|m| m.role == Role::User)
        .map(|m| m.text())
        .collect();
    assert!(
        user_msgs.iter().any(|t| t.contains("analyze code")),
        "prompt text recorded: {user_msgs:?}"
    );
}

// ---------------------------------------------------------------------------
// Regression: claim_* must ignore turn_cancel (livelock guard)
// ---------------------------------------------------------------------------
//
// Before the fix, `claim_steers` / `claim_one_queued` each carried a
// `turn_cancel` cancel-guard arm that short-circuited the DB read when the
// token was fired, while `has_pending_steers` / `has_pending_queues` had NO
// such guard. With a pending input + a fired (but un-reset) turn_cancel,
// `claim_*` reported "empty" while `has_pending_*` reported "pending": the
// drain loop spun on `ConsumeNext` forever (no tool calls => the doom-loop
// guard never tripped => a true unbounded livelock). After the fix, `claim_*`
// only respects the hard (session) cancel, so the input is promoted and the
// loop makes progress. The pre-fired turn_cancel still aborts the first LLM
// attempt (by design), after which run_loop resets it and the real turn runs.

/// Pending Steer + pre-fired `turn_cancel` + drain: the steer must be promoted
/// and the loop must terminate instead of stranding/spinning.
#[tokio::test]
async fn drain_mode_pending_steer_with_fired_turn_cancel_promotes_it() {
    let store = mem_store().await;
    seed(&store, "dr-steer-tc", "act").await;

    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![done_turn("aborted")])
            .with_default(vec![done_turn("ok")]),
    );
    let client: Arc<dyn ChatStream> = mock.clone();

    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "dr-steer-tc",
        resolve_agent("act").unwrap(),
        config(),
        client,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();

    store
        .admit_input(&mk_input("dr-steer-tc", Delivery::Steer, "steered prompt"))
        .await
        .unwrap();

    // Pre-fire turn_cancel BEFORE draining -- no active turn exists, so nothing
    // resets it until the first LLM call aborts and run_loop resets it.
    fire_turn_cancel(session.turn_cancel.as_ref().unwrap());

    let evs = drain_run(&mut session).await;

    assert!(
        evs.iter().any(|e| matches!(e, SessionEvent::Done)),
        "must terminate with Done (no livelock)"
    );
    assert!(
        mock.call_count() >= 1,
        "steer must be promoted -> at least 1 LLM call, got {}",
        mock.call_count()
    );
    let user_msgs: Vec<String> = session
        .messages
        .iter()
        .filter(|m| m.role == Role::User)
        .map(|m| m.text())
        .collect();
    assert!(
        user_msgs.iter().any(|t| t.contains("steered prompt")),
        "steer prompt recorded: {user_msgs:?}"
    );
}

/// Pending Queue + pre-fired `turn_cancel` + drain: the queued prompt must be
/// consumed. This is the path that was a true unbounded livelock before the
/// fix (claim_one_queued said None, has_pending_queues said true).
#[tokio::test]
async fn drain_mode_pending_queue_with_fired_turn_cancel_consumes_it() {
    let store = mem_store().await;
    seed(&store, "dr-queue-tc", "act").await;

    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![done_turn("aborted")])
            .with_default(vec![done_turn("ok")]),
    );
    let client: Arc<dyn ChatStream> = mock.clone();

    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "dr-queue-tc",
        resolve_agent("act").unwrap(),
        config(),
        client,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();

    store
        .admit_input(&mk_input("dr-queue-tc", Delivery::Queue, "queued prompt"))
        .await
        .unwrap();

    // Pre-fire turn_cancel BEFORE draining.
    fire_turn_cancel(session.turn_cancel.as_ref().unwrap());

    let evs = drain_run(&mut session).await;

    assert!(
        evs.iter().any(|e| matches!(e, SessionEvent::Done)),
        "must terminate with Done (no livelock)"
    );
    assert!(
        mock.call_count() >= 1,
        "queue must be consumed -> at least 1 LLM call, got {}",
        mock.call_count()
    );
    let user_msgs: Vec<String> = session
        .messages
        .iter()
        .filter(|m| m.role == Role::User)
        .map(|m| m.text())
        .collect();
    assert!(
        user_msgs.iter().any(|t| t.contains("queued prompt")),
        "queue prompt recorded: {user_msgs:?}"
    );
}

// ---------------------------------------------------------------------------
// HOME isolation for skill discovery (copied from control_cmd.rs)
// ---------------------------------------------------------------------------

static HOME_MUTEX: Mutex<()> = Mutex::new(());

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
