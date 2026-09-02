//! Echo-contract regression: the verbatim user input (with `$skill` tokens)
//! is persisted as `Message.display` while `blocks` carry the post-resolution
//! clean text the LLM consumes. Display surfaces (TUI replay, SPA,
//! `session show`) must show the raw input; the token must never reach the
//! model, and the synthetic `SKILL_TRIGGER` must replay as the user's own
//! words instead of the resolved trigger body.

use std::sync::{Arc, Mutex};

use opencoder_core::{resolve_agent, Config, Role};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient, Usage};
use opencoder_session::{run, skill_resolve::SKILL_TRIGGER, SessionEvent, SessionState};
use opencoder_store::{Delivery, LibsqlStore, SessionInput, SessionMeta, Store};

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

async fn seed(store: &Arc<dyn Store>, id: &str) {
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

/// Serializes tests that mutate process-global HOME (skill discovery reads
/// `~/.opencoder/skills`; seeding is per-test via a temp home).
static HOME_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// RAII: points HOME + XDG_CONFIG_HOME at `home`, restores on drop.
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

fn user_messages(session: &SessionState) -> Vec<&opencoder_core::Message> {
    session
        .messages
        .iter()
        .filter(|m| m.role == Role::User)
        .collect()
}

// ---------------------------------------------------------------------------
// 1. Direct prompt `$review fix the bug`: clean text recorded, verbatim
//    display preserved, token never sent to the model.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn direct_prompt_records_verbatim_display_and_clean_text() {
    let home = tempfile::tempdir().unwrap();
    let _guard = lock_home(home.path());
    opencoder_core::seed_builtin_skills();

    let store = mem_store().await;
    seed(&store, "disp-1").await;
    let mock = Arc::new(MockChatClient::new().push_script(vec![done_turn("ok")]));
    let client: Arc<dyn ChatStream> = mock.clone();
    let mut session = mk_session("disp-1", client, store.clone());

    run(&mut session, "$review fix the bug".into(), |_| {})
        .await
        .unwrap();

    let users = user_messages(&session);
    assert_eq!(users.len(), 1, "exactly one user turn recorded");
    // `extract_skill_tokens` strips the token but leaves the separator
    // space (pre-existing runner contract; the TUI trims its own path).
    assert_eq!(
        users[0].text().trim(),
        "fix the bug",
        "blocks carry clean text"
    );
    assert_eq!(
        users[0].display.as_deref(),
        Some("$review fix the bug"),
        "display carries the verbatim input"
    );
    assert!(
        !users[0].synthetic,
        "text-bearing prompt records as real user input"
    );
    for m in &session.messages {
        assert!(
            !m.text().contains("$review"),
            "the token must never land in any message text"
        );
    }
    for req in mock.requests() {
        let body = format!("{req:?}");
        assert!(
            !body.contains("$review"),
            "the token must never reach the LLM request: {body}"
        );
    }

    // Persisted rows round-trip the display column (v14 schema).
    let loaded = store.load_messages("disp-1").await.unwrap();
    let user_row = loaded.iter().find(|m| m.role == Role::User).unwrap();
    assert_eq!(user_row.display.as_deref(), Some("$review fix the bug"));
}

// ---------------------------------------------------------------------------
// 2. Pure-skill direct submit `$review`: synthetic SKILL_TRIGGER carries the
//    verbatim token as display.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn direct_pure_skill_trigger_display_is_verbatim_token() {
    let home = tempfile::tempdir().unwrap();
    let _guard = lock_home(home.path());
    opencoder_core::seed_builtin_skills();

    let store = mem_store().await;
    seed(&store, "disp-2").await;
    let mock = Arc::new(MockChatClient::new().push_script(vec![done_turn("ok")]));
    let client: Arc<dyn ChatStream> = mock.clone();
    let mut session = mk_session("disp-2", client, store.clone());

    run(&mut session, "$review".into(), |_| {}).await.unwrap();

    let users = user_messages(&session);
    assert_eq!(users.len(), 1);
    assert!(users[0].synthetic, "trigger injected as synthetic");
    assert_eq!(users[0].text(), SKILL_TRIGGER);
    assert_eq!(
        users[0].display.as_deref(),
        Some("$review"),
        "replay must show the user's own `$review`, not the trigger body"
    );
}

// ---------------------------------------------------------------------------
// 3. Queued compound `$review fix the bug` (consumption-time resolution):
//    record_compound keeps the verbatim display, clean text, single turn.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn queued_compound_records_verbatim_display() {
    let home = tempfile::tempdir().unwrap();
    let _guard = lock_home(home.path());
    opencoder_core::seed_builtin_skills();

    let store = mem_store().await;
    seed(&store, "disp-3").await;
    let mock = Arc::new(MockChatClient::new().push_script(vec![done_turn("ok")]));
    let client: Arc<dyn ChatStream> = mock.clone();
    let mut session = mk_session("disp-3", client, store.clone());

    store
        .admit_input(&mk_input("disp-3", Delivery::Queue, "$review fix the bug"))
        .await
        .unwrap();

    let events: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    run(&mut session, String::new(), move |ev| {
        sink.lock().unwrap().push(ev)
    })
    .await
    .unwrap();

    assert!(
        events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, SessionEvent::Done)),
        "drain completed"
    );
    let users = user_messages(&session);
    assert_eq!(users.len(), 1, "queued input recorded exactly once");
    assert_eq!(users[0].text().trim(), "fix the bug");
    assert_eq!(
        users[0].display.as_deref(),
        Some("$review fix the bug"),
        "consumption-time resolution must not lose the verbatim echo"
    );
}
