//! Regression lock: `/task` session switching restores per-session state.
//!
//! Switching between two stored sessions (A: model-x + sandbox, B: model-y +
//! act) must bring back each session's own stored model and agent every time,
//! in both directions:
//!
//! 1. `model_label` is synced from the loaded session's config
//!    (`app_task.rs::switch_session`, `*model_label = new_session.config.model`).
//! 2. The agent comes from `sessions.agent` via `resume`
//!    (`crates/session/src/resume.rs`).
//! 3. `config.model` is restored from `sessions.model`, and the next turn of
//!    the switched-to session runs with that restored model.
//! 4. The chat view is rebuilt from the target session's store transcript and
//!    the interaction state (scroll/follow/queue/skill) is snapshotted and
//!    restored per session, so nothing leaks across sessions.
//!
//! The switch itself (`app_task.rs::switch_session`) is `pub(crate)`; these
//! tests drive its exact building blocks from the public surface: the worker
//! persistence paths (`/agent` prompts + `ReloadConfig`) to seed the rows,
//! `resume()` for the pure load, `worker::process_cmd` for the follow-up turn,
//! and `session_ui` snapshot/replay for the UI round-trip.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use opencoder_core::{resolve_agent, Config, ContentBlock, Message, ProviderConfig};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_session::{resume, SessionState};
use opencoder_store::{LibsqlStore, SessionMeta, Store};
use opencoder_tui::chat::block_text;
use opencoder_tui::session_ui::{replay_into_chat, SessionUiState};
use opencoder_tui::worker::{process_cmd, UiCmd, UiEvent};
use tokio::sync::mpsc;

/// `Config::default().model` -- the live model a session carries before any
/// stored value overrides it.
const BASE_MODEL: &str = "gpt-4o-mini";
const MODEL_A: &str = "prov-x/model-x";
const MODEL_B: &str = "prov-y/model-y";
/// `resume()` installs `config.model_id()` (bare id) as the session model the
/// runner puts on the wire (`crates/session/src/resume.rs`, `llm_call.rs`).
const MODEL_A_BARE: &str = "model-x";
const MODEL_B_BARE: &str = "model-y";
const SESSION_A: &str = "switch-restore-a";
const SESSION_B: &str = "switch-restore-b";

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

fn config_with(model: &str) -> Config {
    Config {
        model: model.into(),
        provider: ProviderConfig {
            api_key: Some("k".into()),
            ..Default::default()
        },
        ..Config::default()
    }
}

/// Drive one command through the FIFO worker and collect its events.
///
/// `process_cmd` awaits its internal event forwarder before returning, so
/// every forwarded event is already buffered in the channel; dropping our
/// sender closes the channel and lets the drain below terminate promptly.
async fn run_cmd(sess: &mut SessionState, cmd: UiCmd) -> Vec<UiEvent> {
    let (tx, mut rx) = mpsc::channel::<UiEvent>(64);
    let quit = process_cmd(cmd, sess, &tx).await;
    assert!(!quit, "worker must not signal quit");
    drop(tx);
    let mut events = Vec::new();
    while let Some(ev) = rx.recv().await {
        events.push(ev);
    }
    events
}

/// Seed a stored session through the SAME persistence paths the TUI uses:
/// `/agent` control prompts persist `sessions.agent` and the `/model`
/// session-only switch dispatches `ReloadConfig`, which the worker persists
/// into `sessions.model`. Asserting the row here keeps the switch tests below
/// pure restore tests (they cannot pass because of sloppy seeding).
async fn seed_session(
    store: &Arc<dyn Store>,
    id: &str,
    from_agent: &str,
    to_agent: &str,
    model: &str,
) {
    store
        .create_session(&SessionMeta {
            id: id.into(),
            agent: Some(from_agent.into()),
            ..Default::default()
        })
        .await
        .unwrap();
    let mut sess = SessionState::new(
        id,
        resolve_agent(from_agent).unwrap(),
        config_with(BASE_MODEL),
        Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>,
        std::env::temp_dir(),
    )
    .with_store(store.clone())
    .mark_session_created();
    if from_agent != to_agent {
        run_cmd(&mut sess, UiCmd::Prompt(format!("/{to_agent}"), vec![])).await;
    }
    run_cmd(&mut sess, UiCmd::ReloadConfig(Box::new(config_with(model)))).await;

    let meta = store
        .get_session(id)
        .await
        .unwrap()
        .expect("seeded session row exists");
    assert_eq!(meta.agent.as_deref(), Some(to_agent), "seeded agent");
    assert_eq!(meta.model.as_deref(), Some(model), "seeded model");
}

/// The `/task` switch load path (`app_task.rs::load_session_for_switch`):
/// `Config::load(workdir)` falls back to the live config when the workdir has
/// no opencoder.json, then `resume()` must override model + agent from the
/// stored row. Returns the loaded session plus the `model_label` the switch
/// arm syncs (`*model_label = new_session.config.model`).
async fn switch_to(
    store: &Arc<dyn Store>,
    id: &str,
    client: Arc<dyn ChatStream>,
    workdir: &Path,
) -> (SessionState, String) {
    let live = Config::load(workdir).unwrap_or_else(|_| config_with(BASE_MODEL));
    let loaded = resume(store.clone(), id, live, client, workdir.to_path_buf())
        .await
        .expect("switch load succeeds");
    let model_label = loaded.config.model.clone();
    (loaded, model_label)
}

fn user_msg(id: &str, text: &str) -> Message {
    let mut m = Message::user(id, "");
    m.blocks = vec![ContentBlock::text(text)];
    m
}

fn assistant_msg(id: &str, text: &str) -> Message {
    let mut m = Message::assistant(id);
    m.blocks.push(ContentBlock::text(text));
    m
}

/// A -> B -> A -> B round-trip: every switch brings back that session's own
/// stored model (label + config + wire id) and agent chip.
#[tokio::test]
async fn switch_restores_model_and_agent_both_ways() {
    let dir = tempfile::tempdir().unwrap();
    let store = mem_store().await;
    seed_session(&store, SESSION_A, "act", "sandbox", MODEL_A).await;
    seed_session(&store, SESSION_B, "sandbox", "act", MODEL_B).await;
    let client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());

    // A -> B: B comes back with ITS OWN stored model and agent.
    let (b, model_label) = switch_to(&store, SESSION_B, client.clone(), dir.path()).await;
    assert_eq!(model_label, MODEL_B, "model_label follows B's stored model");
    assert_eq!(b.config.model, MODEL_B, "in-session config restored");
    assert_eq!(
        b.model, MODEL_B_BARE,
        "resume installs the bare model id the runner puts on the wire"
    );
    assert_eq!(b.agent.name, "act", "agent chip restored for B");

    // B -> A: the first session restores too, no bleed-through of B's values.
    let (a, model_label) = switch_to(&store, SESSION_A, client.clone(), dir.path()).await;
    assert_eq!(model_label, MODEL_A);
    assert_eq!(a.config.model, MODEL_A);
    assert_eq!(a.model, MODEL_A_BARE);
    assert_eq!(a.agent.name, "sandbox", "agent chip restored for A");

    // A -> B again: the round-trip is stable, not just a one-shot restore.
    let (b2, model_label) = switch_to(&store, SESSION_B, client, dir.path()).await;
    assert_eq!(model_label, MODEL_B);
    assert_eq!(b2.config.model, MODEL_B);
    assert_eq!(b2.model, MODEL_B_BARE);
    assert_eq!(b2.agent.name, "act");

    // Switching never rewrites the other row's stored values.
    for (id, agent, model) in [(SESSION_A, "sandbox", MODEL_A), (SESSION_B, "act", MODEL_B)] {
        let meta = store.get_session(id).await.unwrap().unwrap();
        assert_eq!(meta.agent.as_deref(), Some(agent), "stored agent of {id}");
        assert_eq!(meta.model.as_deref(), Some(model), "stored model of {id}");
    }
}

/// After switching to B, the next turn of that session runs against the
/// restored model: the request body the mock LLM captures carries it.
#[tokio::test]
async fn switched_model_used_by_next_turn() {
    let dir = tempfile::tempdir().unwrap();
    let store = mem_store().await;
    seed_session(&store, SESSION_A, "act", "sandbox", MODEL_A).await;
    seed_session(&store, SESSION_B, "sandbox", "act", MODEL_B).await;

    let mock = Arc::new(MockChatClient::new());
    let (mut b, model_label) = switch_to(
        &store,
        SESSION_B,
        mock.clone() as Arc<dyn ChatStream>,
        dir.path(),
    )
    .await;
    assert_eq!(model_label, MODEL_B);

    mock.queue_script(vec![LlmEvent::Completed {
        text: "pong".into(),
        tool_calls: vec![],
        usage: None,
    }]);
    run_cmd(&mut b, UiCmd::Prompt("ping".into(), vec![])).await;

    assert_eq!(
        mock.call_count(),
        1,
        "the post-switch turn hits the LLM once"
    );
    let requests = mock.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].model, MODEL_B_BARE,
        "the next turn after the switch must use the restored model"
    );
}

/// The chat view is rebuilt from the target session's store transcript (never
/// the previous session's), and the interaction state round-trips through the
/// per-session snapshot: drifted values for A come back intact after B.
#[tokio::test]
async fn switch_rebuilds_chat_view_without_cross_talk() {
    let dir = tempfile::tempdir().unwrap();
    let store = mem_store().await;
    seed_session(&store, SESSION_A, "act", "sandbox", MODEL_A).await;
    seed_session(&store, SESSION_B, "sandbox", "act", MODEL_B).await;
    store
        .append_message(SESSION_A, &user_msg("u-a", "prompt for alpha"))
        .await
        .unwrap();
    store
        .append_message(SESSION_A, &assistant_msg("a-a", "alpha reply"))
        .await
        .unwrap();
    store
        .append_message(SESSION_B, &user_msg("u-b", "prompt for beta"))
        .await
        .unwrap();
    store
        .append_message(SESSION_B, &assistant_msg("a-b", "beta reply"))
        .await
        .unwrap();

    let client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());
    let mut session_states: HashMap<String, SessionUiState> = HashMap::new();

    // Live session A, with interaction state deliberately drifted from fresh.
    let (a, _) = switch_to(&store, SESSION_A, client.clone(), dir.path()).await;
    let mut chat = replay_into_chat(&a.agent.name, &a.messages, &store, SESSION_A, 0).await;
    assert_eq!(chat.agent, "sandbox", "live view chip shows A's agent");
    let mut history = vec!["alpha follow-up".to_string()];
    let mut scroll: u32 = 9;
    let mut follow = false;
    let mut queue_scroll: u32 = 4;
    let mut active_skill = Some("code-review".to_string());
    let mut active_skill_body = Some("review carefully".to_string());
    let (running, sys_tokens) = (true, 12_345_u64);
    let mut queue_items: Vec<(i64, String)> = vec![(5, "queued alpha".into())];

    // Switch A -> B: snapshot A, then rebuild for B (first visit: no cache).
    session_states.insert(
        SESSION_A.into(),
        SessionUiState::snapshot(
            running,
            &chat,
            &history,
            scroll,
            follow,
            queue_scroll,
            sys_tokens,
            &queue_items,
            &active_skill,
            &active_skill_body,
        ),
    );
    let (b, _) = switch_to(&store, SESSION_B, client.clone(), dir.path()).await;
    let restored_b = session_states.remove(SESSION_B);
    assert!(restored_b.is_none(), "first visit to B has no cached state");
    chat = replay_into_chat(&b.agent.name, &b.messages, &store, SESSION_B, 0).await;
    // Fresh interaction state for a session visited for the first time.
    scroll = 0;
    follow = true;
    queue_scroll = 0;
    active_skill = None;
    active_skill_body = None;
    queue_items = Vec::new();

    let b_text = block_text(&chat);
    assert!(
        b_text.contains("prompt for beta"),
        "B's own transcript shows"
    );
    assert!(
        b_text.contains("beta reply"),
        "B's own assistant text shows"
    );
    assert!(
        !b_text.contains("alpha"),
        "A's transcript must not bleed into B's rebuilt view, got: {b_text}"
    );
    assert_eq!(chat.agent, "act", "the chip follows the switched-to agent");

    // Switch B -> A: the snapshot taken on the way out is restored.
    session_states.insert(
        SESSION_B.into(),
        SessionUiState::snapshot(
            false,
            &chat,
            &history,
            scroll,
            follow,
            queue_scroll,
            sys_tokens,
            &queue_items,
            &active_skill,
            &active_skill_body,
        ),
    );
    let (a2, _) = switch_to(&store, SESSION_A, client, dir.path()).await;
    let st = session_states
        .remove(SESSION_A)
        .expect("A's snapshot was saved on the way out");
    chat = replay_into_chat(&a2.agent.name, &a2.messages, &store, SESSION_A, 0).await;
    history = st.history;
    scroll = st.scroll;
    follow = st.follow;
    queue_scroll = st.queue_scroll;
    active_skill = st.active_skill;
    active_skill_body = st.active_skill_body;
    queue_items = st.queue_items;

    assert_eq!(scroll, 9, "A's scroll position survives the round-trip");
    assert!(!follow, "A's follow flag survives the round-trip");
    assert_eq!(queue_scroll, 4, "A's queue scroll survives the round-trip");
    assert_eq!(
        history,
        vec!["alpha follow-up".to_string()],
        "A's composer history survives the round-trip"
    );
    assert_eq!(active_skill.as_deref(), Some("code-review"));
    assert_eq!(active_skill_body.as_deref(), Some("review carefully"));
    assert_eq!(queue_items, vec![(5, "queued alpha".into())]);
    assert!(st.running, "the snapshot captured A's running flag");

    let a_text = block_text(&chat);
    assert!(
        a_text.contains("prompt for alpha"),
        "A's transcript is back"
    );
    assert!(
        !a_text.contains("beta"),
        "B's transcript must not bleed back into A's view, got: {a_text}"
    );
    assert_eq!(chat.agent, "sandbox", "the chip is back on A's agent");
}
