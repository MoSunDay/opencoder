use super::support::*;
use opencoder_core::agent::{agent_skill_roots, scope::with_root_sync, tools_paths};
use opencoder_llm::{tool_call::CompletedToolCall, LlmEvent, MockChatClient};
use opencoder_store::SessionMeta;
use serde_json::json;
use std::time::Duration;

#[tokio::test]
async fn http_saved_resources_feed_prompt_skills_executable_tools_and_memory() {
    let client = std::sync::Arc::new(
        MockChatClient::new()
            .push_script(vec![LlmEvent::Completed {
                text: String::new(),
                tool_calls: vec![CompletedToolCall {
                    id: "probe".into(),
                    name: "bash".into(),
                    input: json!({"command":"oc-http-probe"}),
                }],
                usage: None,
            }])
            .with_default(vec![LlmEvent::Completed {
                text: "done".into(),
                tool_calls: vec![],
                usage: None,
            }]),
    );
    let server = Server::start(client.clone()).await;
    server.create("http-probe", json!({})).await;
    server
        .save(
            "http-probe",
            "prompts",
            json!([
                file("soul.md", "HTTP_SOUL"),
                file("how.md", "HTTP_HOW"),
                file("output.md", "HTTP_OUTPUT")
            ]),
        )
        .await;
    server
        .save(
            "http-probe",
            "skills",
            json!([
                file(
                    "probe/SKILL.md",
                    "---\nname: http-probe-skill\ndescription: probe\n---\nHTTP_SKILL"
                ),
                file("probe/assets/data.bin", [0, 255])
            ]),
        )
        .await;
    let mut tool = file("oc-http-probe", "#!/bin/sh\necho HTTP_TOOL\n");
    tool["mode"] = json!(0o755);
    server.save("http-probe", "tools", json!([tool])).await;
    server
        .save(
            "http-probe",
            "memory",
            json!([file("memory.md", "HTTP_MEMORY")]),
        )
        .await;
    let pinned = server.temp.path().join("accepted");
    copy_tree(&server.root, &pinned);
    let old_prompt = with_root_sync(Some(pinned.clone()), || {
        opencoder_core::resolve_agent("http-probe").unwrap().prompt
    });
    assert!(old_prompt.contains("HTTP_SOUL") && old_prompt.contains("# Memory\nHTTP_MEMORY"));
    server
        .save(
            "http-probe",
            "prompts",
            json!([file("soul.md", "HTTP_NEW_SOUL")]),
        )
        .await;
    assert_eq!(
        with_root_sync(Some(pinned), || opencoder_core::resolve_agent("http-probe")
            .unwrap()
            .prompt),
        old_prompt
    );
    server.scoped(|| {
        assert_eq!(agent_skill_roots("http-probe").len(), 1);
        assert_eq!(
            tools_paths(opencoder_core::config::ToolsScope::All, Some("http-probe")).len(),
            1
        );
    });
    server
        .state
        .store
        .create_session(&SessionMeta {
            id: "http-session".into(),
            agent: Some("http-probe".into()),
            autopilot_mode: Some("off".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    let (status, response) = server
        .call(
            "POST",
            "/api/sessions/http-session/prompt",
            Some(json!({"prompt":"$http-probe-skill run the probe"})),
        )
        .await;
    assert_eq!(status, 200, "{response}");
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if !server
                .state
                .handles
                .lock()
                .await
                .get("http-session")
                .is_some_and(|h| h.draining.load(std::sync::atomic::Ordering::SeqCst))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let requests = format!("{:?}", client.requests());
    for expected in [
        "HTTP_NEW_SOUL",
        "HTTP_HOW",
        "HTTP_OUTPUT",
        "HTTP_SKILL",
        "HTTP_MEMORY",
    ] {
        assert!(
            requests.contains(expected),
            "missing {expected}: {requests}"
        );
    }
    let messages = serde_json::to_string(
        &server
            .state
            .store
            .load_messages("http-session")
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(messages.contains("HTTP_TOOL"), "{messages}");
}
