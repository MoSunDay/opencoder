use super::*;
use opencoder_core::harness::Harness;
use opencoder_core::Message;
use opencoder_llm::MockChatClient;
use opencoder_store::LibsqlStore;

// ── pending_replay_hint (pure) ──────────────────────────────────────

#[test]
fn new_task_keeps_active_provider_model_and_loads_other_settings() {
    let active = Config {
        model: "prov-x/model-x".into(),
        ..Config::default()
    };
    let disk = Config {
        model: "disk/default".into(),
        max_tokens: Some(8192),
        ..Config::default()
    };
    let config = new_task_config(disk, &active);
    assert_eq!(config.model, "prov-x/model-x");
    assert_eq!(config.model_id(), "model-x");
    assert_eq!(config.max_tokens, Some(8192));
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
        LibsqlStore::open(dir.path().join("switch.db"))
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
        LibsqlStore::open(dir.path().join("counts.db"))
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

#[path = "switch.rs"]
mod switch;
