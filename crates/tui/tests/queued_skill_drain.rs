//! P0 regression: a `$skill` submission queued while a turn is running
//! activates at **consumption** time (the idle-boundary drain), never at
//! submit time.
//!
//! The TUI admits the raw text (`$haiku fix the bug`, token included) to the
//! queue and touches neither the shared `skill_prompt` Arc nor
//! `sessions.skill`. The still-running kickoff turn therefore carries no
//! `[active skill]` reminder (and no latent-tool unlock); only after the
//! drain consumes the item does `record_compound` resolve + activate +
//! persist the skill, so the drained turn ships the skill body as the
//! `[skill loaded]` message (the transient `[active skill]` tail is
//! fallback-only and stays suppressed while that marker is on record) and
//! the recorded user message is token-stripped.
use std::sync::Arc;

use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{LlmEvent, MockChatClient};
use opencoder_session::SessionState;
use opencoder_store::{Delivery, LibsqlStore, SessionInput, SessionMeta, Store};
use opencoder_tui::worker::{process_cmd, UiCmd, UiEvent};
use tokio::sync::mpsc;

/// Serializes tests that mutate process-global HOME (skill discovery reads
/// `~/.opencoder/skills`). `&'static` mutex => `MutexGuard<'static>`.
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

fn write_haiku_skill(home: &std::path::Path) {
    let dir = home.join(".opencoder").join("skills").join("haiku");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("SKILL.md"), "Always answer in haiku form.").unwrap();
}

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

fn text_done(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![],
        usage: None,
    }
}

fn message_contents(req: &opencoder_llm::ChatRequest) -> Vec<&str> {
    req.messages
        .iter()
        .filter_map(|m| m.get("content").and_then(|c| c.as_str()))
        .collect()
}

fn system_content(req: &opencoder_llm::ChatRequest) -> String {
    req.messages
        .iter()
        .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("system"))
        .and_then(|m| m.get("content").and_then(|c| c.as_str()))
        .unwrap_or("")
        .to_string()
}

fn queue_input(session_id: &str, prompt: &str) -> SessionInput {
    SessionInput {
        seq: None,
        id: "q-1".into(),
        session_id: session_id.into(),
        delivery: Delivery::Queue,
        prompt: prompt.into(),
        images: Vec::new(),
        display_text: Some(prompt.into()),
        admitted_seq: 0,
        promoted_seq: None,
    }
}

#[tokio::test]
async fn queued_skill_fires_at_consumption_not_during_kickoff() {
    let home = tempfile::tempdir().unwrap();
    let _guard = lock_home(home.path());
    write_haiku_skill(home.path());

    let store = mem_store().await;
    store
        .create_session(&SessionMeta {
            id: "q-skill".into(),
            agent: Some("act".into()),
            ..Default::default()
        })
        .await
        .unwrap();

    // Kickoff turn settles; the drained queued follow-up is the 2nd call.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![text_done("t1")])
            .push_script(vec![text_done("t2")]),
    );
    let (tx, _rx) = mpsc::channel::<UiEvent>(64);
    let mut sess = SessionState::new(
        "q-skill",
        resolve_agent("act").expect("act agent"),
        Config::default(),
        mock.clone(),
        std::env::temp_dir(),
    )
    .with_store(store.clone());

    // New TUI admission: the queue row keeps the RAW text — the `$haiku`
    // token is still in the prompt/display, and NO skill was activated or
    // persisted at queue time.
    store
        .admit_input(&queue_input("q-skill", "$haiku fix the bug"))
        .await
        .unwrap();
    assert!(
        store
            .get_session("q-skill")
            .await
            .unwrap()
            .and_then(|m| m.skill)
            .is_none(),
        "no skill persisted while the item merely sits queued"
    );
    assert!(
        sess.skill_prompt_cloned().is_none(),
        "queue admission must not touch the in-memory skill handle"
    );

    // Submit the kickoff; the run drains the queued follow-up at the first
    // idle boundary (no tool calls in turn 1).
    let quit = process_cmd(UiCmd::Prompt("kickoff".into(), vec![]), &mut sess, &tx).await;
    assert!(!quit, "Prompt must not break the worker loop");

    let requests = mock.requests();
    assert!(
        requests.len() >= 2,
        "expected kickoff turn + drained queued follow-up, got {}",
        requests.len()
    );

    // CORE P0: the kickoff turn — running while the `$skill` item sat queued
    // — must carry NO active-skill reminder anywhere, and no skill body in
    // the system prompt.
    for content in message_contents(&requests[0]) {
        assert!(
            !content.contains("[active skill]"),
            "queued $skill must not fire inside the running turn: {content}"
        );
    }
    assert!(
        !system_content(&requests[0]).contains("haiku"),
        "skill bodies never ship in the system prompt: {}",
        system_content(&requests[0])
    );

    // The drained turn resolves the skill at consumption: the body ships
    // as the `[skill loaded]` message naming the source file, and the
    // `[active skill]` tail stays suppressed while that marker is present.
    let drained = &requests[1];
    assert!(
        !system_content(drained).contains("haiku"),
        "drained system prompt stays skill-free"
    );
    let last_user = drained
        .messages
        .iter()
        .rev()
        .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
        .and_then(|m| m.get("content").and_then(|c| c.as_str()))
        .unwrap_or("");
    assert!(
        last_user.contains("[skill loaded]") && last_user.contains("haiku/SKILL.md"),
        "drained queued turn must carry the loaded skill body: {last_user}"
    );
    assert!(
        !last_user.contains("[active skill]"),
        "tail must be suppressed while the loaded marker is present: {last_user}"
    );

    // The recorded user message is the clean text; the token never reaches
    // the model even though the queue row kept it.
    let user_msgs: Vec<&str> = drained
        .messages
        .iter()
        .filter(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
        .filter_map(|m| m.get("content").and_then(|c| c.as_str()))
        .collect();
    assert!(
        user_msgs.iter().any(|c| c.contains("fix the bug")),
        "queued clean text must reach the model: {user_msgs:?}"
    );
    assert!(
        user_msgs.iter().all(|c| !c.contains("$haiku")),
        "the $skill token must never reach the LLM: {user_msgs:?}"
    );

    // One-shot `$skill` semantics (see skill_one_shot.rs): the skill lives
    // exactly for the run that consumed the queued item. Activation at
    // consumption is proven by the drained request's tail reminder above;
    // the run-end hook then clears the skill from memory AND the store, so
    // later runs — and a resume after a clean end — start skill-less.
    // (Mid-run persistence, the crash-resume path, is pinned at the session
    // layer by steer_skill_deferral.rs.)
    let persisted = store
        .get_session("q-skill")
        .await
        .unwrap()
        .and_then(|m| m.skill);
    assert_eq!(
        persisted, None,
        "one-shot clear wiped the persisted skill after run end"
    );
    assert!(
        sess.skill_prompt_cloned().is_none(),
        "one-shot clear wiped the in-memory skill handle after run end"
    );
}
