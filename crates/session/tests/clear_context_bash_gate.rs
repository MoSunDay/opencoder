//! Single-chain evidence for the merged clear-context contract:
//! `/clear_context` no longer switches agents (plan stays plan), so a clear
//! alone must NOT unblock bash writes — the very next turn's mutating write
//! stays BLOCKED ("Blocked in plan mode"; the blocking half of that
//! classification contract is pinned in `bash_guard_plan_mode.rs`). Only an
//! EXPLICIT `/act …` switch unblocks: the targeted directory actually
//! disappears, proving the command ran instead of merely reporting no
//! block, and the switch is what let it through.
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
async fn clear_keeps_plan_gate_then_explicit_act_unblocks() {
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
    // Script for run 1 (post-clear seed turn) + run 2 (blocked write + ack)
    // + run 3 (post-switch write + wrap-up). Order must be exact.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![done_turn("cleared")])
            .push_script(vec![bash_write_turn(
                "bash-1",
                "rm -rf ./opencoder-still-protected",
                &wd,
            )])
            .push_script(vec![done_turn("understood")])
            .push_script(vec![bash_write_turn(
                "bash-2",
                "rm -rf ./opencoder-act-target",
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

    // -- Run 1: the clear itself. Keeps plan, no switch, gate untouched.
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev_clone = events.clone();
    run(&mut session, "/clear_context".into(), move |ev| {
        ev_clone.lock().unwrap().push(ev)
    })
    .await
    .unwrap();
    {
        let evs = events.lock().unwrap();
        assert_eq!(session.agent.name, "plan", "clear keeps the plan agent");
        assert!(
            evs.iter()
                .any(|e| matches!(e, SessionEvent::TranscriptReset(_))),
            "TranscriptReset emitted, got {evs:?}"
        );
        assert!(
            evs.iter()
                .all(|e| !matches!(e, SessionEvent::AgentSwitch(_))),
            "clear must not switch agents, got {evs:?}"
        );
    }
    // Persistence agrees: still plan after the clear.
    let meta = store
        .get_session("plan-clear-bash")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(meta.agent.as_deref(), Some("plan"), "clear persists plan");

    // Sentinel the mutating legs will target: still present after the
    // plan leg, gone after the act leg. Std-fs writes, not bash, so the
    // sentinels exist regardless of the gate.
    let wd_path = workdir.path();
    std::fs::create_dir_all(wd_path.join("opencoder-still-protected")).unwrap();
    std::fs::create_dir_all(wd_path.join("opencoder-act-target")).unwrap();

    // -- Run 2: the very next write is still gated. A clear alone must not
    // unblock bash, exactly because the session never left plan mode.
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev_clone = events.clone();
    run(&mut session, "try a write".into(), move |ev| {
        ev_clone.lock().unwrap().push(ev)
    })
    .await
    .unwrap();
    {
        let evs = events.lock().unwrap();
        assert_eq!(session.agent.name, "plan", "write attempt stays plan");
        let tool_idx = evs
            .iter()
            .position(|e| matches!(e, SessionEvent::ToolEnd { name, .. } if name == "bash"))
            .expect("bash ToolEnd emitted");
        if let SessionEvent::ToolEnd {
            is_error, output, ..
        } = &evs[tool_idx]
        {
            assert!(*is_error, "plan gate must block the write, output: {output}");
            assert!(
                output.contains("Blocked in plan mode"),
                "denial must name plan mode, got: {output}"
            );
            assert!(
                output.contains("/agent act"),
                "denial must point at the real escape hatch, got: {output}"
            );
        }
    }
    assert_eq!(mock.call_count(), 3, "clear + blocked write + ack turns");
    assert!(
        workdir.path().join("opencoder-still-protected").is_dir(),
        "the gated write must never have run: the sentinel survives"
    );

    // -- Run 3: the EXPLICIT `/act …` switch is what unblocks. The write
    // executes for real (its effect lands in the call workdir).
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev_clone = events.clone();
    run(&mut session, "/act make the write".into(), move |ev| {
        ev_clone.lock().unwrap().push(ev)
    })
    .await
    .unwrap();
    {
        let evs = events.lock().unwrap();
        assert_eq!(session.agent.name, "act", "explicit /act switched to act");
        let switch_idx = evs
            .iter()
            .position(|e| matches!(e, SessionEvent::AgentSwitch(a) if a == "act"))
            .expect("AgentSwitch(act) emitted");
        let tool_idx = evs
            .iter()
            .position(|e| matches!(e, SessionEvent::ToolEnd { name, .. } if name == "bash"))
            .expect("bash ToolEnd emitted");
        assert!(
            switch_idx < tool_idx,
            "the write must execute strictly after the switch, got {evs:?}"
        );
        if let SessionEvent::ToolEnd {
            is_error, output, ..
        } = &evs[tool_idx]
        {
            assert!(!*is_error, "act agent must not gate bash, output: {output}");
            assert!(
                !output.contains("Blocked in plan mode"),
                "post-switch act agent must not gate bash, output: {output}"
            );
        }
    }

    // The command really executed: its effect landed in the call workdir.
    assert!(
        !workdir.path().join("opencoder-act-target").exists(),
        "bash write must have actually run in the call workdir"
    );

    // The explicit switch persisted for resume.
    let meta = store
        .get_session("plan-clear-bash")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(meta.agent.as_deref(), Some("act"), "switch persists");
}
