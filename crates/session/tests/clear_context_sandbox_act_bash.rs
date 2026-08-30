//! Single-chain evidence for the sandbox -> act clear-context convergence:
//! the convergence must actually UNBLOCK bash writes. Within one `run`, a
//! sandbox session clears context (TranscriptReset then AgentSwitch(act))
//! and the very next turn executes a REAL cwd-relative write command. cwd is
//! outside the sandbox release set, so a session that stayed sandbox would
//! have blocked it (that half is pinned in `bash_guard_sandbox_mode.rs`);
//! the created directory proves the command actually ran instead of merely
//! reporting no block.

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config, ContentBlock, Message};
use opencoder_llm::{CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::{run, SessionEvent, SessionState};
use opencoder_store::{LibsqlStore, SessionMeta, Store};

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

/// Assistant turn issuing one real bash write call (cwd-relative, so it
/// targets the session tempdir and nothing outside test control).
fn bash_write_turn(cmd: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: String::new(),
        tool_calls: vec![CompletedToolCall {
            id: "bash-1".into(),
            name: "bash".into(),
            input: serde_json::json!({ "command": cmd }),
        }],
        usage: Some(Usage::default()),
    }
}

async fn seed(store: &Arc<dyn Store>, id: &str) {
    store
        .create_session(&SessionMeta {
            id: id.into(),
            agent: Some("sandbox".into()),
            model: Some("m/g".into()),
            created_at: 0,
            updated_at: 0,
            ..Default::default()
        })
        .await
        .unwrap();
}

/// Assistant reply that makes the transcript seed-flavoured, so the clear
/// falls through to a real execution turn (where the bash write happens).
fn assistant_say(id: &str, text: &str) -> Message {
    let mut m = Message::assistant(id);
    m.blocks.push(ContentBlock::text(text));
    m
}

#[tokio::test]
async fn sandbox_clear_then_real_bash_write_executes_unblocked() {
    let store = mem_store().await;
    seed(&store, "sandbox-clear-bash").await;
    let msgs = vec![
        Message::user("u1", "old question"),
        assistant_say("a1", "old answer"),
    ];
    store
        .append_messages("sandbox-clear-bash", &msgs)
        .await
        .unwrap();

    // Turn 1 (the post-clear seed-execution turn) issues the real write;
    // turn 2 wraps the run up.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![bash_write_turn(
                "mkdir -p ./opencoder-clear-act-write",
            )])
            .push_script(vec![done_turn("write done")]),
    );
    // The tempdir must outlive the run: the bash tool resolves relative
    // paths against this session working directory.
    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "sandbox-clear-bash",
        resolve_agent("sandbox").unwrap(),
        config(),
        mock.clone(),
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();
    session.messages = msgs.clone();

    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev_clone = events.clone();
    run(&mut session, "/act_clear_context".into(), move |ev| {
        ev_clone.lock().unwrap().push(ev)
    })
    .await
    .unwrap();

    {
        let evs = events.lock().unwrap();
        assert_eq!(session.agent.name, "act", "sandbox clear converges to act");
        assert_eq!(mock.call_count(), 2, "write turn + wrap-up turn ran");

        let reset_idx = evs
            .iter()
            .position(|e| matches!(e, SessionEvent::TranscriptReset(_)))
            .expect("TranscriptReset emitted");
        let switch_idx = evs
            .iter()
            .position(|e| matches!(e, SessionEvent::AgentSwitch(a) if a == "act"))
            .expect("AgentSwitch(act) emitted");
        let tool_idx = evs
            .iter()
            .position(|e| matches!(e, SessionEvent::ToolEnd { name, .. } if name == "bash"))
            .expect("bash ToolEnd emitted");
        assert!(
            reset_idx < switch_idx && switch_idx < tool_idx,
            "the write must execute strictly after the convergence, got {evs:?}"
        );
        if let SessionEvent::ToolEnd {
            is_error, output, ..
        } = &evs[tool_idx]
        {
            assert!(!*is_error, "write must not error, output: {output}");
            assert!(
                !output.contains("Blocked in sandbox mode"),
                "post-convergence act agent must not gate bash, output: {output}"
            );
        }
    }

    // The command really executed: its effect exists in the session cwd.
    assert!(
        dir.path().join("opencoder-clear-act-write").is_dir(),
        "bash write must have actually run in the session cwd"
    );

    // Convergence persisted for resume.
    let meta = store
        .get_session("sandbox-clear-bash")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(meta.agent.as_deref(), Some("act"), "convergence persists");
}
