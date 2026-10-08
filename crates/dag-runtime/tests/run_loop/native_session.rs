use super::*;

#[tokio::test]
async fn act_step_with_local_memory_uses_runner_skills_inside_the_container() {
    let (base, shared) = spawn_stub().await;
    let temporary = tempfile::tempdir().unwrap();
    let client = Arc::new(
        MockChatClient::new()
            .push_script(vec![LlmEvent::Completed {
                text: "{\"checked\":true}".into(),
                tool_calls: vec![],
                usage: None,
            }])
            .push_script(vec![LlmEvent::Completed {
                text: "Memory checked; no changes required.".into(),
                tool_calls: vec![],
                usage: None,
            }]),
    );
    let mut fixture = fixture(&base, &temporary, client.clone()).await;
    fixture.config.local_memory = true;
    let mut spec = one_step_spec();
    spec.steps[0].name = "memory".into();
    let run = claimed(spec);
    let run_id = run.run_id.clone();
    let (_, cancel) = tokio::sync::watch::channel(false);
    let status = execute_run(
        RunDeps {
            uplink: fixture.uplink.clone(),
            exec: ExecDeps {
                store: fixture.store.clone(),
                workdir: fixture.workdir.clone(),
                config: fixture.config.clone(),
            },
            workflow_root: fixture.workflow_root.clone(),
        },
        run,
        cancel,
    )
    .await
    .unwrap();
    assert_eq!(
        status,
        DagRunStatus::Done,
        "events: {:?}",
        shared.lock().unwrap().events
    );
    await_status(&shared).await;
    assert_eq!(
        client.call_count(),
        2,
        "task and local memory must both run"
    );
    // The merged workspace is unmounted at completion. Check retained overlay
    // files and the archived public output instead of the empty mountpoint.
    let run_root = fixture.workflow_root.join(&run_id);
    let written = run_root.join("upper/memory");
    assert!(written
        .join(".opencoder/runtime/global-skills/repo-local-memory/SKILL.md")
        .is_file());
    let output: serde_json::Value =
        serde_json::from_slice(&std::fs::read(run_root.join("memory/output.json")).unwrap())
            .unwrap();
    assert_eq!(output, json!({"checked":true}));
    let events = std::fs::read_to_string(written.join("events.ndjson")).unwrap();
    assert!(events.contains("local memory updated"), "{events}");
    assert!(!events.contains("skill is missing"), "{events}");
}

#[tokio::test]
async fn agent_tools_reasoning_and_messages_survive_the_shared_container() {
    let (base, shared) = spawn_stub().await;
    let temporary = tempfile::tempdir().unwrap();
    let client = Arc::new(MockChatClient::new()
        .push_script(vec![
            LlmEvent::ReasoningDelta("inspect the workspace".into()),
            LlmEvent::Completed {
                text: String::new(),
                tool_calls: vec![opencoder_llm::CompletedToolCall {
                    id: "workspace-probe".into(),
                    name: "bash".into(),
                    input: json!({"command":"set -e; pwd; printf 'shared file' > note.txt; cat note.txt; git --version; python3 -c 'print(\"native python\")'"}),
                }],
                usage: None,
            },
        ])
        .push_script(vec![LlmEvent::Completed {
            text: "{\"checked\":true}".into(),
            tool_calls: vec![],
            usage: None,
        }]));
    let fixture = fixture(&base, &temporary, client.clone()).await;
    let mut spec = one_step_spec();
    spec.steps[0].name = "agent".into();
    let run = claimed(spec);
    let run_id = run.run_id.clone();
    let (_, cancel) = tokio::sync::watch::channel(false);
    let status = execute_run(
        RunDeps {
            uplink: fixture.uplink.clone(),
            exec: ExecDeps {
                store: fixture.store.clone(),
                workdir: fixture.workdir.clone(),
                config: fixture.config.clone(),
            },
            workflow_root: fixture.workflow_root.clone(),
        },
        run,
        cancel,
    )
    .await
    .unwrap();
    assert_eq!(status, DagRunStatus::Done);
    await_status(&shared).await;
    assert_eq!(client.call_count(), 2);
    let meta: serde_json::Value = serde_json::from_slice(
        &std::fs::read(fixture.workflow_root.join(&run_id).join("agent/meta.json")).unwrap(),
    )
    .unwrap();
    let session = meta["session_id"].as_str().unwrap();
    let messages = fixture.store.load_messages(session).await.unwrap();
    assert!(
        messages
            .iter()
            .any(|message| message.role == opencoder_core::Role::Tool
                && message.blocks.iter().any(|block| matches!(block,
                    opencoder_core::ContentBlock::ToolResult { content, is_error: false, .. }
                    if content.contains("/workspace/agent") && content.contains("shared file")
                    && content.contains("git version") && content.contains("native python")))),
        "{messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|message| message.text().contains("checked")),
        "{messages:?}"
    );
    let captured = shared.lock().unwrap();
    for (kind, expected) in [
        ("reasoning_delta", "inspect the workspace"),
        ("tool_start", "workspace-probe"),
        ("tool_end", "shared file"),
    ] {
        assert!(
            captured.events.iter().any(|event| event.kind == "step_log"
                && event.step.as_deref() == Some("agent")
                && event.payload["event"] == kind
                && event.payload["data"].to_string().contains(expected)),
            "{kind}"
        );
    }
}
