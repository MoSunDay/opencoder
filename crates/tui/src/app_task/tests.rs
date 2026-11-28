use super::*;
use opencoder_core::harness::Harness;
use opencoder_core::{Message, ProviderConfig};
use opencoder_llm::{ChatRequest, MockChatClient};
use opencoder_store::TursoStore;

// ── pending_replay_hint (pure) ──────────────────────────────────────

#[test]
fn new_task_keeps_current_shared_model_providers_and_other_settings() {
    let active = Config {
        model: "outer/stale".into(),
        ..Config::default()
    };
    let disk = Config {
        model: "disk/default".into(),
        max_tokens: Some(8192),
        ..Config::default()
    };
    let config = new_task_config(disk, &active, "prov-x/model-x");
    assert_eq!(config.model, "prov-x/model-x");
    assert_eq!(config.model_id(), "model-x");
    assert_eq!(config.max_tokens, Some(8192));
}

#[test]
fn new_task_empty_shared_label_preserves_legacy_fallback() {
    let active = Config {
        model: "legacy/shared".into(),
        ..Config::default()
    };
    let config = new_task_config(Config::default(), &active, "");
    assert_eq!(config.model, "legacy/shared");
}

fn provider(protocol: &str, temperature: Option<f64>) -> ProviderConfig {
    ProviderConfig {
        protocol: protocol.into(),
        base_url: "http://127.0.0.1:9".into(),
        api_key: Some("test-key".into()),
        temperature,
        ..Default::default()
    }
}

fn routing_config() -> Config {
    let mut config = Config {
        model: "main/shared".into(),
        ..Default::default()
    };
    config
        .providers
        .insert("main".into(), provider("chat_completions", None));
    config
}

fn route_request(model: &str) -> ChatRequest {
    ChatRequest {
        purpose: opencoder_llm::RequestPurpose::Conversation,
        model: model.into(),
        messages: Vec::new(),
        tools: Vec::new(),
        tool_choice: None,
        temperature: None,
        max_tokens: None,
        reasoning_effort: None,
        cache_salt: None,
    }
}

#[test]
fn new_task_client_routes_with_added_edited_and_deleted_providers() {
    let disk = Config {
        model: "disk/default".into(),
        max_tokens: Some(4096),
        ..Default::default()
    };
    let mut active = routing_config();
    active
        .providers
        .insert("target".into(), provider("responses", None));

    let added = new_task_config(disk.clone(), &active, "main/shared");
    assert_eq!(added.max_tokens, Some(4096));
    assert_eq!(added.providers, active.providers);
    let added_client = new_task_client(&added).unwrap();
    let added_body = added_client
        .request_body(&route_request("target/added-model"))
        .unwrap();
    assert_eq!(added_body["model"], "added-model");
    assert!(added_body.get("input").is_some());
    assert!(added_body.get("messages").is_none());

    active
        .providers
        .insert("target".into(), provider("chat_completions", Some(0.25)));
    let edited = new_task_config(disk.clone(), &active, "main/shared");
    let edited_client = new_task_client(&edited).unwrap();
    let edited_body = edited_client
        .request_body(&route_request("target/edited-model"))
        .unwrap();
    assert_eq!(edited_body["model"], "edited-model");
    assert_eq!(edited_body["temperature"], 0.25);
    assert!(edited_body.get("messages").is_some());

    active.providers.remove("target");
    let deleted = new_task_config(disk, &active, "main/shared");
    assert!(!deleted.providers.contains_key("target"));
    let deleted_client = new_task_client(&deleted).unwrap();
    let error = deleted_client
        .request_body(&route_request("target/deleted-model"))
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("unknown model provider `target`"),
        "deleted provider must not survive in the new client's snapshot: {error:#}"
    );
}

#[test]
fn new_task_keeps_codex_selection_and_injected_environment() {
    let opts = TuiOpts::new(None)
        .with_harness(
            Some(Harness::Codex),
            [("CODEX_HOME".into(), "/tmp/codex-auth".into())]
                .into_iter()
                .collect(),
        )
        .with_model(Some("codex-model".into()));
    let mut session = SessionState::new(
        "fresh-codex-task",
        resolve_agent("act").unwrap(),
        Config::default(),
        Arc::new(MockChatClient::new()),
        std::env::temp_dir(),
    );
    session.harness = new_task_harness(session.harness.harness, &opts);
    assert_eq!(session.harness.harness, Harness::Codex);
    assert_eq!(session.harness.envs["CODEX_HOME"], "/tmp/codex-auth");
    assert_eq!(session.harness.model.as_deref(), Some("codex-model"));
    assert!(session.harness.thread_id.is_none());
}

#[test]
fn pending_replay_hint_none_for_zero() {
    assert_eq!(pending_replay_hint(0), None, "no pending -> no marker");
}

#[test]
fn pending_replay_hint_lists_count_and_trigger() {
    let one = pending_replay_hint(1).expect("n>0 yields a hint");
    assert!(one.contains("1 subagent(s)"), "got: {one}");
    assert!(one.contains("replay pending"), "got: {one}");
    assert!(one.contains("next message"), "got: {one}");
    let three = pending_replay_hint(3).expect("n>0 yields a hint");
    assert!(three.contains("3 subagent(s)"), "got: {three}");
}

// ── load_session_for_switch (pure load, no replay) ──────────────────

fn user_msg(id: &str, text: &str) -> Message {
    Message {
        provider_state: None,
        display: None,
        id: id.into(),
        role: opencoder_core::Role::User,
        blocks: vec![opencoder_core::ContentBlock::text(text)],
        model: None,
        agent: None,
        usage: opencoder_core::MessageUsage::default(),
        created_at: 0,
        synthetic: false,
    }
}

fn assistant_task_use(id: &str, tool_use_id: &str) -> Message {
    Message {
        provider_state: None,
        display: None,
        id: id.into(),
        role: opencoder_core::Role::Assistant,
        blocks: vec![opencoder_core::ContentBlock::ToolUse {
            id: tool_use_id.into(),
            name: "task".into(),
            input: serde_json::json!({"prompt": "explore"}),
        }],
        model: None,
        agent: None,
        usage: opencoder_core::MessageUsage::default(),
        created_at: 0,
        synthetic: false,
    }
}

/// The switch path must be a PURE data load: a Cancelled subagent stays
/// Cancelled (no eager LLM replay), its dangling `task` tool_use stays
/// dangling in the parent transcript (no synthetic error tool_result —
/// the next-turn replay will answer it), child messages are untouched,
/// and the pending count is reported for the hint marker.
#[tokio::test]
async fn load_session_for_switch_is_pure_load_no_replay() {
    let dir = tempfile::tempdir().unwrap();
    let store: Arc<dyn Store> = Arc::new(
        TursoStore::open(dir.path().join("switch.db"))
            .await
            .unwrap(),
    );
    let client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());

    // Parent session: user prompt + assistant dangling `task` tool_use.
    for sid in ["parent", "child-x"] {
        store
            .create_session(&opencoder_store::SessionMeta {
                id: sid.into(),
                title: Some(sid.into()),
                agent: Some("act".into()),
                model: Some("m".into()),
                created_at: 0,
                updated_at: 0,
                ..Default::default()
            })
            .await
            .unwrap();
    }
    store
        .append_messages(
            "parent",
            &[
                user_msg("u1", "explore the repo"),
                assistant_task_use("a1", "task-1"),
            ],
        )
        .await
        .unwrap();
    store
        .append_message("child-x", &user_msg("c-u1", "child working"))
        .await
        .unwrap();
    store
        .create_subagent_task(&opencoder_store::SubagentTaskRecord {
            task_id: "task-1".into(),
            parent_session_id: "parent".into(),
            child_session_id: "child-x".into(),
            parent_message_id: Some("a1".into()),
            agent: "explore".into(),
            prompt: "explore the repo".into(),
            result: None,
            status: opencoder_store::SubagentStatus::Cancelled,
            ok: None,
            started_at: 0,
            completed_at: None,
        })
        .await
        .unwrap();

    let before_parent = store.load_messages("parent").await.unwrap();
    let before_child = store.load_messages("child-x").await.unwrap();

    let (session, pending) =
        load_session_for_switch(&store, "parent", Config::default(), &client, dir.path())
            .await
            .unwrap();

    // Pending count feeds the hint marker.
    assert_eq!(pending, 1, "one cancelled task must be reported pending");

    // Task untouched: still Cancelled, no result backfilled.
    let task = store.get_subagent_task("task-1").await.unwrap().unwrap();
    assert_eq!(
        task.status,
        opencoder_store::SubagentStatus::Cancelled,
        "switch must not replay the cancelled task"
    );
    assert!(task.result.is_none(), "no replay result may be backfilled");

    // Parent transcript unchanged: the dangling tool_use is still the
    // last word (no synthetic Tool message answering it on load).
    let after_parent = store.load_messages("parent").await.unwrap();
    assert_eq!(
        after_parent.len(),
        before_parent.len(),
        "pure load must not append messages to the parent"
    );
    let dangling_kept = session
            .messages
            .last()
            .map(|m| {
                matches!(m.role, opencoder_core::Role::Assistant)
                    && m.blocks
                        .iter()
                        .any(|b| matches!(b, opencoder_core::ContentBlock::ToolUse { id, .. } if id == "task-1"))
            })
            .unwrap_or(false);
    assert!(
        dangling_kept,
        "replayable dangling tool_use must survive the load unanswered"
    );

    // Child transcript unchanged.
    let after_child = store.load_messages("child-x").await.unwrap();
    assert_eq!(
        after_child.len(),
        before_child.len(),
        "pure load must not touch the child transcript"
    );
}

/// No pending tasks -> count 0 (no marker); Running tasks count as
/// pending too, and Completed ones do not.
#[tokio::test]
async fn load_session_for_switch_counts_only_pending_statuses() {
    let dir = tempfile::tempdir().unwrap();
    let store: Arc<dyn Store> = Arc::new(
        TursoStore::open(dir.path().join("counts.db"))
            .await
            .unwrap(),
    );
    let client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());
    store
        .create_session(&opencoder_store::SessionMeta {
            id: "p".into(),
            ..Default::default()
        })
        .await
        .unwrap();

    let mk = |task_id: &str, child: &str, status| opencoder_store::SubagentTaskRecord {
        task_id: task_id.into(),
        parent_session_id: "p".into(),
        child_session_id: child.into(),
        parent_message_id: None,
        agent: "explore".into(),
        prompt: "p".into(),
        result: None,
        status,
        ok: None,
        started_at: 0,
        completed_at: None,
    };
    for sid in ["c1", "c2", "c3"] {
        store
            .create_session(&opencoder_store::SessionMeta {
                id: sid.into(),
                ..Default::default()
            })
            .await
            .unwrap();
    }
    store
        .create_subagent_task(&mk("t1", "c1", opencoder_store::SubagentStatus::Running))
        .await
        .unwrap();
    store
        .create_subagent_task(&mk("t2", "c2", opencoder_store::SubagentStatus::Cancelled))
        .await
        .unwrap();
    store
        .create_subagent_task(&mk("t3", "c3", opencoder_store::SubagentStatus::Completed))
        .await
        .unwrap();

    let (_session, pending) =
        load_session_for_switch(&store, "p", Config::default(), &client, dir.path())
            .await
            .unwrap();
    assert_eq!(
        pending, 2,
        "Running + Cancelled count; Completed is terminal and must not"
    );
}

/// A failed switch must name the session it was resuming: the store-level
/// cause (here: no such session) is wrapped with the `/task` resume context
/// so a bare resume error cannot reach the switch handler unidentified.
#[tokio::test]
async fn load_session_for_switch_error_names_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let store: Arc<dyn Store> = Arc::new(
        TursoStore::open(dir.path().join("switch-missing.db"))
            .await
            .unwrap(),
    );
    let client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());

    let error = match load_session_for_switch(
        &store,
        "absent-session",
        Config::default(),
        &client,
        dir.path(),
    )
    .await
    {
        Ok(_) => panic!("an unknown session must fail the switch"),
        Err(error) => format!("{error:#}"),
    };
    assert!(
        error.contains("task switch resume for session absent-session"),
        "error names the switch and the session: {error}"
    );
    assert!(
        error.contains("session not found: absent-session"),
        "the store-level cause stays in the chain: {error}"
    );
}

/// A failed switch on the REMOTE path must be identified the same way: the
/// remote probe runs before the local resume, so a session tagged as remote
/// without a bound harness runtime fails the bookmark invariant and must be
/// wrapped with the `/task` remote-load context, never the resume one.
#[tokio::test]
async fn load_session_for_switch_error_names_the_session_on_the_remote_path() {
    let dir = tempfile::tempdir().unwrap();
    let store: Arc<dyn Store> = Arc::new(
        TursoStore::open(dir.path().join("switch-remote-broken.db"))
            .await
            .unwrap(),
    );
    let client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());
    store
        .create_session(&opencoder_store::SessionMeta {
            id: "agent-broken".into(),
            kind: Some("tui_remote_agent".into()),
            ..Default::default()
        })
        .await
        .unwrap();

    let error = match load_session_for_switch(
        &store,
        "agent-broken",
        Config::default(),
        &client,
        dir.path(),
    )
    .await
    {
        Ok(_) => panic!("a remote session without a binding must fail the switch"),
        Err(error) => format!("{error:#}"),
    };
    assert!(
        error.contains("task switch remote load for session agent-broken"),
        "error names the switch, the remote path and the session: {error}"
    );
    assert!(
        error.contains("Remote task bookmark is incomplete"),
        "the remote-level cause stays in the chain: {error}"
    );
    assert!(
        !error.contains("task switch resume for session"),
        "the local resume path must not be blamed: {error}"
    );
}

/// `switch_error` returns the text the flash line shows and keeps the whole
/// cause chain, with and without a session id on the pick.
#[test]
fn switch_error_keeps_the_cause_chain() {
    let error = anyhow::Error::new(std::io::Error::other("damaged row"))
        .context("decode sessions.created_at for session s1");
    let text = switch_error(&crate::task::TaskPick::Resume("s1".into()), &error);
    assert_eq!(text, format!("{error:#}"));
    assert!(
        text.contains("s1"),
        "the chain still names the session: {text}"
    );

    let plain = switch_error(&crate::task::TaskPick::New, &error);
    assert_eq!(plain, format!("{error:#}"));
}

#[path = "switch.rs"]
mod switch;

#[path = "resume_new.rs"]
mod resume_new;

#[path = "explicit_model.rs"]
mod explicit_model;

#[path = "restored_active.rs"]
mod restored_active;
