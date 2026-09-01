//! One-shot `$skill` semantics — integration contract tests.
//!
//! An activation (inline `$name` token, or a pre-set/resumed
//! `skill_prompt`) lives ONLY for the run (`run_loop` invocation) that
//! triggered it. When that run ends — Done, Error, or cancel — the skill is
//! cleared from memory (`skill_prompt` + `active_skill_names`) and from the
//! store (`clear_skill`), so subsequent runs start skill-less: no
//! `[active skill]` tail reminder, no `[skill loaded]` body payload (it was
//! transient per-call anyway — run end stops the submission entirely), no
//! resumed-session resurrection. The single deliberate exception: a crash
//! MID-run leaves `sessions.skill` set, so the resumed run KEEPS the skill
//! until it completes — then the same run-end clear lands.
//! Latent tools unlocked by the skill body re-lock at the same boundary.

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config, Role};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_session::{resume, run, SessionState};
use opencoder_store::{LibsqlStore, SessionMeta, SessionPatch, Store};
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------
// fixtures (mirror tests/plain_skill_prompt.rs / tests/skill_queue_drain.rs)
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
        usage: Some(opencoder_llm::Usage::default()),
    }
}

/// An exhausted mock (no script, no default) makes `chat_stream` itself
/// return Err — the LLM-failure shape for the Err-clears test.
fn failing_client() -> Arc<dyn ChatStream> {
    Arc::new(MockChatClient::new())
}

async fn seed(store: &Arc<dyn Store>, id: &str) {
    store
        .create_session(&SessionMeta {
            id: id.into(),
            agent: Some("act".into()),
            model: Some("m".into()),
            created_at: 0,
            updated_at: 0,
            ..SessionMeta::default()
        })
        .await
        .unwrap();
}

fn mk_session(id: &str, client: Arc<dyn ChatStream>, store: Arc<dyn Store>) -> SessionState {
    let dir = tempfile::tempdir().unwrap();
    SessionState::new(
        id,
        resolve_agent("act").unwrap(),
        config(),
        client,
        dir.path().to_path_buf(),
    )
    .with_store(store)
    .mark_session_created()
}

// ---- HOME isolation for `$alpha` discovery (mirrors other skill suites) ----

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

/// Write a discoverable `alpha` skill into `home/.opencoder/skills`.
fn write_alpha_skill(home: &std::path::Path) {
    let dir = home.join(".opencoder").join("skills").join("alpha");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("SKILL.md"),
        "---\nname: alpha\ndescription: test skill\n---\nALPHA-BODY\n",
    )
    .unwrap();
}

/// Assert the one-shot postcondition: no skill in memory (body + names) and
/// none persisted on the session row.
async fn assert_cleared(session: &SessionState, store: &Arc<dyn Store>, id: &str) {
    assert!(
        session.skill_prompt_cloned().is_none(),
        "memory skill cleared"
    );
    assert!(
        session.active_skill_names_cloned().is_empty(),
        "active_skill_names cleared"
    );
    let meta = store
        .get_session(id)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("session row {id} exists"));
    assert!(
        meta.skill.is_none(),
        "store skill cleared, got {:?}",
        meta.skill
    );
}

// ---------------------------------------------------------------------------
// 1. Done clears
// ---------------------------------------------------------------------------

/// `$alpha do the thing` runs WITH the skill (tail reminder + the transient
/// `[skill loaded]` body payload during the run), and the Ok return leaves
/// the skill cleared in memory AND on the store row.
#[tokio::test]
async fn done_clears_skill_after_run() {
    let home = tempfile::tempdir().unwrap();
    let _guard = lock_home(home.path());
    write_alpha_skill(home.path());

    let store = mem_store().await;
    seed(&store, "one-shot-done").await;
    let mock = Arc::new(MockChatClient::new().push_script(vec![done_turn("did it")]));
    let client: Arc<dyn ChatStream> = mock.clone();
    let mut s = mk_session("one-shot-done", client, store.clone());

    run(&mut s, "$alpha do the thing".into(), |_| {})
        .await
        .unwrap();

    // The run itself really used the skill before clearing it.
    assert_eq!(
        s.messages
            .iter()
            .find(|m| m.role == Role::User && !m.synthetic)
            .map(|m| m.text()),
        Some(" do the thing".into()),
        "token stripped, prompt recorded"
    );
    assert!(
        user_texts(&mock.requests()[0])
            .iter()
            .any(|t| t.starts_with("[skill loaded] ")),
        "body shipped in the run's payload"
    );
    assert_eq!(mock.call_count(), 1, "exactly one LLM round");

    assert_cleared(&s, &store, "one-shot-done").await;
}

// ---------------------------------------------------------------------------
// 2. Err clears
// ---------------------------------------------------------------------------

/// An LLM failure (exhausted mock) mid-run propagates as Err AND leaves the
/// skill cleared — a failed run must not strand the activation for the next
/// run or for a resume.
#[tokio::test]
async fn llm_error_clears_skill_after_run() {
    let home = tempfile::tempdir().unwrap();
    let _guard = lock_home(home.path());
    write_alpha_skill(home.path());

    let store = mem_store().await;
    seed(&store, "one-shot-err").await;
    let mut s = mk_session("one-shot-err", failing_client(), store.clone());

    let res = run(&mut s, "$alpha do the thing".into(), |_| {}).await;
    assert!(res.is_err(), "exhausted mock surfaces as run Err");

    assert_cleared(&s, &store, "one-shot-err").await;
}

// ---------------------------------------------------------------------------
// 3. Cancel clears
// ---------------------------------------------------------------------------

/// A pre-tripped cancel token breaks the loop before the first LLM round
/// ("interrupted"); the run returns Ok and the skill is cleared all the
/// same — cancel is just another run end.
#[tokio::test]
async fn cancel_clears_skill_after_run() {
    let home = tempfile::tempdir().unwrap();
    let _guard = lock_home(home.path());
    write_alpha_skill(home.path());

    let store = mem_store().await;
    seed(&store, "one-shot-cancel").await;
    let mock = Arc::new(MockChatClient::new().push_script(vec![done_turn("unreached")]));
    let client: Arc<dyn ChatStream> = mock.clone();
    let mut s = mk_session("one-shot-cancel", client, store.clone());
    let token = CancellationToken::new();
    token.cancel();
    s.cancel = Some(token);

    run(&mut s, "$alpha do the thing".into(), |_| {})
        .await
        .unwrap();
    assert_eq!(mock.call_count(), 0, "cancel short-circuits the LLM round");

    assert_cleared(&s, &store, "one-shot-cancel").await;
}

// ---------------------------------------------------------------------------
// 4. No-skill run is a no-op
// ---------------------------------------------------------------------------

/// A plain (skill-less) run must not touch the skill state at all: the
/// guard in `clear_on_run_end` returns before any store write.
#[tokio::test]
async fn no_skill_run_keeps_skill_none() {
    let store = mem_store().await;
    seed(&store, "one-shot-plain").await;
    let mock = Arc::new(MockChatClient::new().push_script(vec![done_turn("plain reply")]));
    let client: Arc<dyn ChatStream> = mock.clone();
    let mut s = mk_session("one-shot-plain", client, store.clone());

    run(&mut s, "just work".into(), |_| {}).await.unwrap();

    assert!(s.skill_prompt_cloned().is_none(), "still skill-less");
    assert!(s.active_skill_names_cloned().is_empty());
    let meta = store
        .get_session("one-shot-plain")
        .await
        .unwrap()
        .expect("row exists");
    assert!(meta.skill.is_none(), "store stays skill-less");
}

// ---------------------------------------------------------------------------
// 5. Subsequent say carries no reminder
// ---------------------------------------------------------------------------

/// After the skill run completes, a second plain prompt must ship with NO
/// `[active skill]` tail and NO `[skill loaded]` body at all — the payload
/// message was transient per-call, so run end stopped its submission
/// entirely (nothing persisted to replay).
#[tokio::test]
async fn second_run_has_no_skill_reminder() {
    let home = tempfile::tempdir().unwrap();
    let _guard = lock_home(home.path());
    write_alpha_skill(home.path());

    let store = mem_store().await;
    seed(&store, "one-shot-second").await;
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![done_turn("skill work")])
            .push_script(vec![done_turn("plain work")]),
    );
    let client: Arc<dyn ChatStream> = mock.clone();
    let mut s = mk_session("one-shot-second", client, store.clone());

    run(&mut s, "$alpha do the thing".into(), |_| {})
        .await
        .unwrap();
    let first = mock.requests()[0].clone();
    assert!(
        user_texts(&first)
            .iter()
            .any(|t| t.starts_with("[skill loaded] ")),
        "run 1 receives the armed skill via the [skill loaded] message"
    );

    run(&mut s, "plain follow up".into(), |_| {}).await.unwrap();
    let second = mock.requests()[1].clone();
    assert!(
        !user_texts(&second)
            .iter()
            .any(|t| t.contains("[active skill]")),
        "run 2 must carry no [active skill] tail: {:?}",
        user_texts(&second)
    );
    assert_eq!(
        loaded_marker_count(&second),
        0,
        "run 2 carries NO [skill loaded] body — run end stopped the transient \
         submission entirely"
    );

    assert_cleared(&s, &store, "one-shot-second").await;
}

/// String contents of every `user` message in a captured request.
fn user_texts(req: &opencoder_llm::ChatRequest) -> Vec<String> {
    req.messages
        .iter()
        .filter(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
        .filter_map(|m| m.get("content").and_then(|c| c.as_str()))
        .map(str::to_string)
        .collect()
}

/// User messages of a request whose text starts with the `[skill loaded]`
/// marker line (the transient full-body payload message).
fn loaded_marker_count(req: &opencoder_llm::ChatRequest) -> usize {
    user_texts(req)
        .iter()
        .filter(|t| t.starts_with("[skill loaded] "))
        .count()
}

// ---------------------------------------------------------------------------
// 6. Resume mid-run keeps, completion clears
// ---------------------------------------------------------------------------

/// A session row with `sessions.skill` set simulates a crash MID-run: the
/// resumed session restores the skill (the run-end clear never landed), the
/// resumed run keeps it for its own turn, and completing that run clears it
/// in memory and on the row.
#[tokio::test]
async fn resume_mid_run_keeps_skill_then_completion_clears() {
    let store = mem_store().await;
    seed(&store, "one-shot-resume").await;
    store
        .update_session(
            "one-shot-resume",
            &SessionPatch {
                skill: Some("> Source: /skills/alpha/SKILL.md\n\nALPHA-BODY".into()),
                updated_at: Some(0),
                ..SessionPatch::default()
            },
        )
        .await
        .unwrap();

    let mock = Arc::new(MockChatClient::new().push_script(vec![done_turn("resumed work")]));
    let client: Arc<dyn ChatStream> = mock.clone();
    let dir = tempfile::tempdir().unwrap();
    let mut s = resume(
        store.clone(),
        "one-shot-resume",
        config(),
        client,
        dir.path().to_path_buf(),
    )
    .await
    .unwrap();

    assert!(
        s.skill_prompt_cloned().is_some(),
        "mid-run crash resume keeps the skill (restore half)"
    );

    run(&mut s, "continue".into(), |_| {}).await.unwrap();

    // The resumed run did use the skill before its own end cleared it.
    let first = mock.requests()[0].clone();
    assert!(
        user_texts(&first)
            .iter()
            .any(|t| t.starts_with("[skill loaded] ")),
        "resumed run carried the skill (loaded message)"
    );

    assert_cleared(&s, &store, "one-shot-resume").await;
}

// ---------------------------------------------------------------------------
// 7. Latent tools re-lock at run end
// ---------------------------------------------------------------------------

/// A latent tool (`ssh_pty`) unlocked by the skill body appears in the
/// triggering run's tool schemas and disappears again in the next run: the
/// one-shot clear re-locks latent tools together with the skill. Uses a
/// custom agent with `ToolFilter::All` so the latent filter is the only gate
/// (the default `act` allowlist would hide ssh_pty regardless).
#[tokio::test]
async fn latent_tool_unlocked_then_relocked_across_runs() {
    let home = tempfile::tempdir().unwrap();
    let _guard = lock_home(home.path());
    let skill_dir = home
        .path()
        .join(".opencoder")
        .join("skills")
        .join("ssh-pty");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: ssh-pty\ndescription: test skill\n---\nUse ssh_pty for persistent SSH.\n",
    )
    .unwrap();

    let store = mem_store().await;
    seed(&store, "one-shot-latent").await;
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![done_turn("ssh work")])
            .push_script(vec![done_turn("plain work")]),
    );
    let client: Arc<dyn ChatStream> = mock.clone();
    let agent = opencoder_core::Agent {
        name: "act".into(),
        kind: opencoder_core::AgentKind::Act,
        mode: opencoder_core::AgentMode::Primary,
        description: String::new(),
        prompt: String::new(),
        tools: opencoder_core::ToolFilter::All,
    };
    let dir = tempfile::tempdir().unwrap();
    let mut s = SessionState::new(
        "one-shot-latent",
        agent,
        config(),
        client,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();

    run(&mut s, "$ssh-pty connect".into(), |_| {})
        .await
        .unwrap();
    let first = mock.requests()[0].clone();
    let first_tools = tool_names(&first);
    assert!(
        first_tools.contains(&"ssh_pty".to_string()),
        "run 1 unlocked the latent tool: {first_tools:?}"
    );

    run(&mut s, "plain follow up".into(), |_| {}).await.unwrap();
    let second = mock.requests()[1].clone();
    let second_tools = tool_names(&second);
    assert!(
        !second_tools.contains(&"ssh_pty".to_string()),
        "run 2 must re-lock the latent tool: {second_tools:?}"
    );
    assert!(
        !user_texts(&second)
            .iter()
            .any(|t| t.contains("[active skill]")),
        "run 2 carries no skill tail"
    );

    assert_cleared(&s, &store, "one-shot-latent").await;
}

/// Names of the tool schemas shipped with a captured request.
fn tool_names(req: &opencoder_llm::ChatRequest) -> Vec<String> {
    req.tools
        .iter()
        .filter_map(|t| t["function"]["name"].as_str())
        .map(str::to_string)
        .collect()
}
