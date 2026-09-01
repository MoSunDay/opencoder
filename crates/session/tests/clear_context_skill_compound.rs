//! `/act_clear_context <tail>` compound ordering, pinned end to end.
//!
//! Mechanism under test (observed, not invented): `split_control_prefix`
//! only peels the command token off and keeps the tail verbatim — `$skill`
//! resolution happens at CONSUMPTION time. On the queue path
//! (`runner/drain.rs`, idle-boundary claim) the order is:
//!
//! ```text
//! claim -> control_cmd::apply(ClearContext)   // transcript -> seed marker,
//!                                             // skill: memory None + store NULL
//!      -> skill_resolve::record_compound(rest) // extract_skill_tokens +
//!                                              // discover_skills() (~/.opencoder/skills)
//!                                              // -> re-arm + persist_active_skill
//!      -> DrainOutcome::Prompt -> LLM turn     // payload carries the
//!                                             // transient [skill loaded] body
//!      -> run end -> clear_on_run_end          // memory None + store NULL again
//! ```
//!
//! A tail WITHOUT a `$token` (e.g. `/act_clear_context review`) therefore
//! never re-arms anything: the stale skill armed before the command must not
//! leak across the clear boundary, and the run-end hook has nothing left to
//! clear. Harness mirrors `compound_cmd.rs` / `skill_tail_cleared_after_run_end.rs`.

use std::sync::Arc;

use opencoder_core::{Config, Role, resolve_agent};
use opencoder_llm::{ChatRequest, ChatStream, LlmEvent, MockChatClient, Usage};
use opencoder_session::{SessionState, run};
use opencoder_store::{Delivery, LibsqlStore, SessionInput, SessionMeta, SessionPatch, Store};

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

async fn seed_session(store: &Arc<dyn Store>, id: &str) {
    store
        .create_session(&SessionMeta {
            id: id.into(),
            agent: Some("act".into()),
            model: Some("m/g".into()),
            created_at: 0,
            updated_at: 0,
            ..Default::default()
        })
        .await
        .unwrap();
}

fn mk_input(session_id: &str, prompt: &str) -> SessionInput {
    SessionInput {
        seq: None,
        id: opencoder_session::runner::new_id(),
        session_id: session_id.into(),
        delivery: Delivery::Queue,
        prompt: prompt.into(),
        images: vec![],
        display_text: None,
        admitted_seq: 0,
        promoted_seq: None,
    }
}

/// User-role text of every message in a captured request payload.
fn user_texts(req: &ChatRequest) -> Vec<String> {
    req.messages
        .iter()
        .filter(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
        .filter_map(|m| m.get("content").and_then(|c| c.as_str()))
        .map(str::to_string)
        .collect()
}

/// The one-shot activation proof: the transient `[skill loaded]` full-body
/// payload message or the transient `[active skill]` tail pointer.
fn carries_skill_artifact(req: &ChatRequest) -> bool {
    user_texts(req)
        .iter()
        .any(|t| t.contains("[skill loaded]") || t.contains("[active skill]"))
}

/// Memory + store both clean, shared by the closing assertion of each variant.
async fn assert_skill_gone(s: &SessionState, store: &Arc<dyn Store>, id: &str, ctx: &str) {
    assert!(
        s.skill_prompt_cloned().is_none(),
        "{ctx}: in-memory skill_prompt must be None"
    );
    let meta = store
        .get_session(id)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("{ctx}: session row must exist"));
    assert!(
        meta.skill.is_none(),
        "{ctx}: store row `skill` must be NULL, got {:?}",
        meta.skill
    );
}

/// Queue a compound `/act_clear_context <tail>` behind an active kickoff run,
/// drive both turns, and hand back the observed state. Shared by both tests.
/// The `_dir` tempdir backs `session.working_dir` and must outlive the
/// assertions, so it is returned here.
#[allow(clippy::type_complexity)]
async fn run_kickoff_then_compound(
    id: &str,
    compound: &str,
    scripts: Vec<Vec<LlmEvent>>,
    stale_skill: Option<&str>,
) -> (
    SessionState,
    Arc<dyn Store>,
    Arc<MockChatClient>,
    tempfile::TempDir,
) {
    let store = mem_store().await;
    seed_session(&store, id).await;
    let mut client = MockChatClient::new();
    for script in scripts {
        client = client.push_script(script);
    }
    let mock = Arc::new(client);
    let client: Arc<dyn ChatStream> = mock.clone();
    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        id,
        resolve_agent("act").unwrap(),
        config(),
        client,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();
    // A skill armed before the command (memory + store row, e.g. left by the
    // previous run's crash): arm it here so the kickoff turn carries the
    // artifact and the clear boundary has something to wipe. BOTH layers are
    // armed — the store-row half is what makes the closing NULL assertion
    // prove that the clear actually persisted (`clear_skill: true`).
    if let Some(body) = stale_skill {
        session.set_skill(Some(body.to_string()));
        store
            .update_session(
                id,
                &SessionPatch {
                    skill: Some(body.to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
    }

    store.admit_input(&mk_input(id, compound)).await.unwrap();
    run(&mut session, "kickoff".into(), |_| {}).await.unwrap();
    (session, store, mock, dir)
}

// ---------------------------------------------------------------------------
// (1) ClearContext apply clears the armed skill; a $-less tail never re-arms.
// ---------------------------------------------------------------------------

/// A skill armed before the command (memory + store row, e.g. left by the
/// previous run's crash) must not leak across the clear boundary: apply()
/// wipes it (memory `set_skill(None)` + store `clear_skill: true`), and the
/// `$`-less tail resolves no token, so the tail run's payload is skill-free.
/// The post-run NULL store row is apply's `persist_clear` — the run-end hook
/// no-ops on a skill-less session without writing.
#[tokio::test]
async fn clear_context_clears_armed_skill_and_dollar_less_tail_does_not_rearm() {
    const STALE_BODY: &str =
        "> Source: /skills/rev/SKILL.md\n\nREV-BODY: stale skill must not leak.";
    let (session, store, mock, _dir) = run_kickoff_then_compound(
        "cc-compound-stale",
        "/act_clear_context plan the rollout",
        vec![
            vec![done_turn("kickoff done")],
            vec![done_turn("plan reply")],
        ],
        Some(STALE_BODY),
    )
    .await;

    // The kickoff ran, then the compound was consumed as one queue item.
    let requests = mock.requests();
    assert_eq!(
        requests.len(),
        2,
        "kickoff turn + tail turn, no extra LLM calls"
    );
    assert!(
        carries_skill_artifact(&requests[0]),
        "fixture: the stale skill is armed during the kickoff turn (pre-clear)"
    );
    // The tail turn is skill-free: no stale body, no [skill loaded] injection,
    // no [active skill] tail.
    let tail_texts = user_texts(&requests[1]);
    assert!(
        !tail_texts.iter().any(|t| t.contains("REV-BODY")),
        "stale skill body must not leak into the tail run: {tail_texts:?}"
    );
    assert!(
        !carries_skill_artifact(&requests[1]),
        "a $-less tail never re-arms: no skill artifact in the tail payload"
    );
    // The tail text was recorded as a real prompt (not the raw command).
    assert!(
        session
            .messages
            .iter()
            .any(|m| m.role == Role::User && !m.synthetic && m.text().contains("plan the rollout")),
        "tail text must be recorded as the user prompt"
    );
    assert!(
        !session
            .messages
            .iter()
            .any(|m| m.role == Role::User && m.text().contains("/act_clear_context")),
        "the raw command must never reach the transcript"
    );
    assert_skill_gone(&session, &store, "cc-compound-stale", "after the tail run").await;
}

// ---------------------------------------------------------------------------
// (2)+(3) `$task-plan` tail re-arms at consumption; run-end clear lands NULL.
// ---------------------------------------------------------------------------

/// `/act_clear_context $task-plan <text>` queued: the skill apply clears the
/// context, then the tail's `$task-plan` token is resolved at consumption
/// time against the seeded `~/.opencoder/skills` — the tail turn's payload
/// carries the task-plan body (`[skill loaded]` injection), and when the run
/// ends the run-end clear wipes the re-armed skill from memory AND the store
/// row (which `persist_active_skill` had re-armed before that turn). HOME is
/// isolated to a tempdir with the real built-in seeds so discovery is
/// deterministic and never reads the developer's `~`.
#[tokio::test]
async fn dollar_tail_rearms_task_plan_then_run_end_clears_store() {
    let home = tempfile::tempdir().unwrap();
    let _guard = lock_home(home.path());
    opencoder_core::seed_builtin_skills();

    let (session, store, mock, _dir) = run_kickoff_then_compound(
        "cc-compound-rearm",
        "/act_clear_context $task-plan plan the release",
        vec![
            vec![done_turn("kickoff done")],
            vec![done_turn("plan reply")],
        ],
        None,
    )
    .await;

    let requests = mock.requests();
    assert_eq!(requests.len(), 2, "kickoff turn + tail turn");
    let tail_texts = user_texts(&requests[1]);
    assert!(
        tail_texts.iter().any(|t| t.contains("[skill loaded]")),
        "task-plan must re-arm at consumption: expected the transient \
         [skill loaded] body message in the tail payload, got {tail_texts:?}"
    );
    assert!(
        tail_texts
            .iter()
            .any(|t| t.contains("task-plan") && t.contains("[skill loaded]")),
        "the injected skill must be task-plan itself"
    );
    // The token is stripped; the remaining text is the recorded prompt.
    assert!(
        session
            .messages
            .iter()
            .any(|m| m.role == Role::User && m.text().contains("plan the release")),
        "tail text (token stripped) recorded as the user prompt"
    );
    assert!(
        !session
            .messages
            .iter()
            .any(|m| m.role == Role::User && m.text().contains("$task-plan")),
        "$task-plan token must be stripped from the recorded prompt"
    );

    // (3) one-shot: the re-armed skill is cleared when the run ends — from
    // memory and from the store row it had been persisted into mid-run.
    assert_skill_gone(
        &session,
        &store,
        "cc-compound-rearm",
        "after the re-arm run",
    )
    .await;
}

// ---------------------------------------------------------------------------
// HOME isolation (mirrors compound_cmd.rs): process-global HOME mutation is
// serialized and restored.
// ---------------------------------------------------------------------------

static HOME_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
