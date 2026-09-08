#![cfg(unix)]
#[path = "harness/fixtures.rs"]
mod fixtures;
use opencoder_core::{harness::Harness, Config, ContentBlock};
use opencoder_session::{run, SessionEvent};
use serde_json::Value;

#[tokio::test]
async fn codex_binary_stream_persistence_resume_and_fork() {
    let root = tempfile::tempdir().unwrap();
    let (mut session, store) = fixtures::session(root.path()).await;
    session
        .harness
        .envs
        .insert("EXAMPLE".into(), " 空格=x \nnext".into());
    let mut events = Vec::new();
    run(&mut session, "需求 ' $(literal)\n第二行".into(), |e| {
        events.push(e)
    })
    .await
    .unwrap();
    assert!(events
        .iter()
        .any(|e| matches!(e, SessionEvent::ReasoningDelta(t) if t == "inspect first")));
    assert!(events
        .iter()
        .any(|e| matches!(e, SessionEvent::ToolEnd{output,..} if output == "tool result")));
    let messages = store.load_messages(&session.id).await.unwrap();
    assert!(messages
        .iter()
        .flat_map(|m| &m.blocks)
        .any(|b| matches!(b,ContentBlock::ToolResult{content,..} if content == "tool result")));
    assert_eq!(
        messages.iter().map(|m| m.usage.total_tokens).sum::<u64>(),
        17
    );
    let runtime = store.harness_runtime(&session.id).await.unwrap().unwrap();
    assert_eq!(runtime.thread_id.as_deref(), Some("fixture-thread"));
    assert!(!runtime.in_flight);
    let mut resumed = opencoder_session::resume(
        store.clone(),
        &session.id,
        Config::default(),
        session.client.clone(),
        root.path().into(),
    )
    .await
    .unwrap();
    assert_eq!(resumed.harness.harness, Harness::Codex);
    run(&mut resumed, "follow up".into(), |_| {}).await.unwrap();
    let fork = opencoder_session::fork::fork_session(store.as_ref(), &session.id)
        .await
        .unwrap();
    let mut forked = opencoder_session::resume(
        store.clone(),
        &fork,
        Config::default(),
        session.client.clone(),
        root.path().into(),
    )
    .await
    .unwrap();
    run(&mut forked, "fork requirement".into(), |_| {})
        .await
        .unwrap();
    let records: Vec<Value> = std::fs::read_to_string(root.path().join("capture.jsonl"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert!(records[0]["prompt"]
        .as_str()
        .unwrap()
        .ends_with("需求 ' $(literal)\n第二行"));
    assert_eq!(records[0]["env"], " 空格=x \nnext");
    assert_eq!(records[1]["args"][1], "resume");
    assert_eq!(records[1]["args"][2], "fixture-thread");
    assert_eq!(records[1]["prompt"], "follow up");
    assert_eq!(records[2]["args"][1], "fork");
    assert_eq!(
        store
            .harness_runtime(&fork)
            .await
            .unwrap()
            .unwrap()
            .thread_id
            .as_deref(),
        Some("fork-thread")
    );
    assert_eq!(
        store
            .harness_runtime(&session.id)
            .await
            .unwrap()
            .unwrap()
            .thread_id
            .as_deref(),
        Some("fixture-thread")
    );
}

#[tokio::test]
async fn codex_reads_pinned_agent_files_and_executable_tools() {
    let root = tempfile::tempdir().unwrap();
    let resources = tempfile::tempdir().unwrap();
    fixtures::card(resources.path());
    let (mut session, store) = fixtures::session(root.path()).await;
    session.config.agent.agents_dir = Some(resources.path().into());
    session.agent =
        opencoder_core::agent::scope::with_root_sync(Some(resources.path().into()), || {
            opencoder_core::resolve_agent("custom").unwrap()
        });
    run(&mut session, "read resources".into(), |_| {})
        .await
        .unwrap();
    let snapshot = session.harness.resource_root.clone().unwrap();
    assert!(snapshot.starts_with(root.path().canonicalize().unwrap()));
    assert!(snapshot.join("skills/shared/v1/inspect/SKILL.md").is_file());
    std::fs::write(
        resources.path().join("prompts/shared/v1/soul.md"),
        "REPLACED",
    )
    .unwrap();
    let mut resumed = opencoder_session::resume(
        store,
        &session.id,
        session.config.clone(),
        session.client.clone(),
        root.path().into(),
    )
    .await
    .unwrap();
    run(&mut resumed, "follow up".into(), |_| {}).await.unwrap();
    assert!(resumed.agent.prompt.contains("SOUL_FIXTURE"));
    assert!(!resumed.agent.prompt.contains("REPLACED"));
}

#[tokio::test]
async fn codex_malformed_stream_and_missing_terminal_fail() {
    for mode in ["malformed", "missing_end"] {
        let root = tempfile::tempdir().unwrap();
        let (mut session, _) = fixtures::session(root.path()).await;
        session.harness.envs.insert("FAIL_MODE".into(), mode.into());
        let mut events = Vec::new();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            run(&mut session, "test".into(), |e| events.push(e)),
        )
        .await
        .unwrap();
        assert!(result.is_err(), "{mode}");
        assert!(!events.iter().any(|e| matches!(e, SessionEvent::Done)));
    }
}

#[tokio::test]
async fn codex_cancel_reaps_descendants_and_closes_open_tools() {
    let root = tempfile::tempdir().unwrap();
    let (mut session, _) = fixtures::session(root.path()).await;
    session
        .harness
        .envs
        .insert("FAIL_MODE".into(), "hang".into());
    let child_pid = root.path().join("child.pid");
    session
        .harness
        .envs
        .insert("CHILD_PID".into(), child_pid.display().to_string());
    let token = tokio_util::sync::CancellationToken::new();
    session.cancel = Some(token.clone());
    let path = child_pid.clone();
    let cancellation = tokio::spawn(async move {
        for _ in 0..200 {
            if path.exists() {
                token.cancel();
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("child did not start");
    });
    let mut events = Vec::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        run(&mut session, "test".into(), |e| events.push(e)),
    )
    .await
    .unwrap()
    .unwrap();
    cancellation.await.unwrap();
    assert!(events
        .iter()
        .any(|e| matches!(e, SessionEvent::ToolEnd { is_error: true, .. })));
    assert!(!session.harness.in_flight);
    let pid = std::fs::read_to_string(child_pid).unwrap();
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"));
    assert!(stat.is_err() || stat.unwrap().split_whitespace().nth(2) == Some("Z"));
}

#[tokio::test]
async fn codex_steer_interrupts_then_resumes_and_drains_queue() {
    use opencoder_store::{Delivery, SessionInput};
    let root = tempfile::tempdir().unwrap();
    let (mut session, store) = fixtures::session(root.path()).await;
    session
        .harness
        .envs
        .insert("FAIL_MODE".into(), "steer".into());
    let marker = root.path().join("child.pid");
    session
        .harness
        .envs
        .insert("CHILD_PID".into(), marker.display().to_string());
    let turn_cancel = session.turn_cancel.clone().unwrap();
    let queue_store = store.clone();
    let sid = session.id.clone();
    let steering = tokio::spawn(async move {
        for _ in 0..200 {
            if marker.exists() {
                for (id, prompt, delivery) in [
                    ("steer-id", "new direction", Delivery::Steer),
                    ("queue-id", "queued direction", Delivery::Queue),
                ] {
                    queue_store
                        .admit_input(&SessionInput {
                            id: id.into(),
                            session_id: sid.clone(),
                            prompt: prompt.into(),
                            delivery,
                            admitted_seq: opencoder_core::message::now_ms(),
                            seq: None,
                            images: vec![],
                            display_text: None,
                            promoted_seq: None,
                        })
                        .await
                        .unwrap();
                }
                opencoder_session::fire_turn_cancel(&turn_cancel);
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("Codex child did not start");
    });
    let mut events = vec![];
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        run(&mut session, "first direction".into(), |e| events.push(e)),
    )
    .await
    .unwrap()
    .unwrap();
    steering.await.unwrap();
    assert!(events
        .iter()
        .any(|e| matches!(e,SessionEvent::SteerConsumed{text,..} if text=="new direction")));
    assert!(events
        .iter()
        .any(|e| matches!(e,SessionEvent::QueueConsumed{text,..} if text=="queued direction")));
    let records: Vec<Value> = std::fs::read_to_string(root.path().join("capture.jsonl"))
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert_eq!(records.len(), 3);
    assert_eq!(records[1]["prompt"], "new direction");
    assert_eq!(records[2]["prompt"], "queued direction");
    assert!(store
        .pending_inputs(&session.id, Delivery::Queue)
        .await
        .unwrap()
        .is_empty());
}
