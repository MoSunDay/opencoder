//! Single-chain evidence for the plan execution handoff: the clear preserves
//! the plan, switches to act, and therefore unblocks the next mutating bash
//! call. The targeted directory actually disappears, proving the command ran.
//!
//! The write legs use `rm -rf`, which the shellguard classifier denies even
//! inside the `/tmp` release scope (the workdir tempdir lives under the
//! crate tree, itself under /tmp) — the same shape as
//! `bash_guard_plan_mode.rs::plan_mode_blocks_write_command`.

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

/// A workdir OUTSIDE the /tmp release scope: the crate tree may sit under
/// /tmp (which the shellguard releases wholesale), so the gating legs need
/// a plain directory anchored on $HOME instead.
fn plain_workdir() -> tempfile::TempDir {
    let home = std::env::var("HOME").expect("$HOME set");
    tempfile::Builder::new()
        .prefix("clear-plan-workdir-")
        .tempdir_in(home)
        .expect("writable $HOME for a non-released workdir")
}

fn done_turn(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![],
        usage: Some(Usage::default()),
    }
}

/// Assistant turn issuing one real bash write call, resolved against the
/// per-call workdir so it targets nothing outside test control.
fn bash_write_turn(id: &str, cmd: &str, workdir: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: String::new(),
        tool_calls: vec![CompletedToolCall {
            id: id.into(),
            name: "bash".into(),
            input: serde_json::json!({ "command": cmd, "workdir": workdir }),
        }],
        usage: Some(Usage::default()),
    }
}

async fn seed(store: &Arc<dyn Store>, id: &str) {
    store
        .create_session(&SessionMeta {
            id: id.into(),
            agent: Some("plan".into()),
            model: Some("m/g".into()),
            created_at: 0,
            updated_at: 0,
            ..Default::default()
        })
        .await
        .unwrap();
}

/// Assistant reply that makes the transcript seed-flavoured, so the clear
/// falls through to a real execution turn.
fn assistant_say(id: &str, text: &str) -> Message {
    let mut m = Message::assistant(id);
    m.blocks.push(ContentBlock::text(text));
    m
}

#[tokio::test]
async fn clear_switches_to_act_and_unblocks_next_write() {
    let store = mem_store().await;
    seed(&store, "plan-clear-bash").await;
    let msgs = vec![
        Message::user("u1", "old question"),
        assistant_say("a1", "old answer"),
    ];
    store
        .append_messages("plan-clear-bash", &msgs)
        .await
        .unwrap();

    // The write's per-call workdir is a PLAIN directory (outside the /tmp
    // release scope): the gate classifies relative writes against it, so a
    // session that stayed plan must block the mutating call. Must outlive
    // the run: the sentinels created inside it carry the actual-ran proof.
    let workdir = plain_workdir();
    let wd = workdir.path().to_str().unwrap().to_string();
    // Script for run 1 (post-handoff execution) + run 2 (write + wrap-up).
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![done_turn("cleared")])
            .push_script(vec![bash_write_turn(
                "bash-1",
                "rm -rf ./opencoder-still-protected",
                &wd,
            )])
            .push_script(vec![done_turn("write done")]),
    );
    // The tempdir must outlive the run: the bash tool falls back to this
    // session working directory only when a call carries no workdir.
    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "plan-clear-bash",
        resolve_agent("plan").unwrap(),
        config(),
        mock.clone(),
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();
    session.messages = msgs.clone();

    // -- Run 1: preserve the plan and switch before executing it.
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev_clone = events.clone();
    run(&mut session, "/act_clear_context".into(), move |ev| {
        ev_clone.lock().unwrap().push(ev)
    })
    .await
    .unwrap();
    {
        let evs = events.lock().unwrap();
        assert_eq!(session.agent.name, "act", "clear converges to act");
        assert!(
            evs.iter()
                .any(|e| matches!(e, SessionEvent::TranscriptReset(_))),
            "TranscriptReset emitted, got {evs:?}"
        );
        assert!(evs
            .iter()
            .any(|e| matches!(e, SessionEvent::AgentSwitch(name) if name == "act")));
    }
    // Persistence agrees: resume cannot resurrect plan mode.
    let meta = store
        .get_session("plan-clear-bash")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(meta.agent.as_deref(), Some("act"), "clear persists act");

    // Sentinel the mutating leg will target. Std-fs creates it; bash must
    // remove it under act.
    let wd_path = workdir.path();
    std::fs::create_dir_all(wd_path.join("opencoder-still-protected")).unwrap();

    // -- Run 2: the next write is allowed because the handoff left plan.
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev_clone = events.clone();
    run(&mut session, "try a write".into(), move |ev| {
        ev_clone.lock().unwrap().push(ev)
    })
    .await
    .unwrap();
    {
        let evs = events.lock().unwrap();
        assert_eq!(session.agent.name, "act", "write runs under act");
        let tool_idx = evs
            .iter()
            .position(|e| matches!(e, SessionEvent::ToolEnd { name, .. } if name == "bash"))
            .expect("bash ToolEnd emitted");
        if let SessionEvent::ToolEnd {
            is_error, output, ..
        } = &evs[tool_idx]
        {
            assert!(!*is_error, "act write must run, output: {output}");
            assert!(
                !output.contains("Blocked in plan mode"),
                "act write must not hit the plan gate: {output}"
            );
        }
    }
    assert_eq!(mock.call_count(), 3, "clear + write + completion turns");
    assert!(
        !workdir.path().join("opencoder-still-protected").exists(),
        "bash write must execute in the call workdir"
    );

    // The handoff switch remains persisted.
    let meta = store
        .get_session("plan-clear-bash")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(meta.agent.as_deref(), Some("act"));
}
