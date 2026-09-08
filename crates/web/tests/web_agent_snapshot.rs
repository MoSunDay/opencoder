use axum::{body::Body, http::Request};
use opencoder_core::agent::scope::with_root;
use opencoder_llm::tool_call::CompletedToolCall;
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_store::{LibsqlStore, SessionMeta, Store};
use serde_json::json;
use std::{path::Path, sync::Arc, time::Duration};
use tower::ServiceExt;

fn seed(root: &Path, marker: &str) {
    for (category, relative, content) in [
        ("prompts", "soul.md", format!("{marker}_PROMPT")),
        (
            "skills",
            "probe/SKILL.md",
            format!("---\nname: snapshot-probe\ndescription: probe\n---\n{marker}_SKILL"),
        ),
        (
            "tools",
            "oc-resource-probe",
            format!("#!/bin/sh\nprintf '{marker}_TOOL\\n'\n"),
        ),
    ] {
        let resource = root.join(category).join("probe");
        let file = resource.join("v1").join(relative);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(
            resource.join("meta.json"),
            json!({"name":"probe","current":1}).to_string(),
        )
        .unwrap();
        std::fs::write(&file, content).unwrap();
        #[cfg(unix)]
        if category == "tools" {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    std::fs::create_dir_all(root.join("probe")).unwrap();
    std::fs::write(
        root.join("probe/meta.json"),
        json!({"name":"probe","current":{"prompt":"probe","skills":"probe","tools":"probe"}})
            .to_string(),
    )
    .unwrap();
}

async fn check(scoped: bool, missing: bool) {
    let temp = tempfile::tempdir().unwrap();
    let live = temp.path().join("live");
    let pinned = temp.path().join("pinned");
    seed(&live, "LIVE");
    if !missing {
        seed(&pinned, "PINNED");
    }
    let workdir = temp.path().join("work");
    std::fs::create_dir_all(workdir.join(".opencoder")).unwrap();
    std::fs::write(
        workdir.join("opencoder.json"),
        json!({"agent":{"agents_dir":live},"model":"mock/test"}).to_string(),
    )
    .unwrap();
    std::fs::write(workdir.join(".opencoder/ap.json"), r#"{"mode":"off"}"#).unwrap();
    let client = Arc::new(
        MockChatClient::new()
            .push_script(vec![LlmEvent::Completed {
                text: String::new(),
                tool_calls: vec![CompletedToolCall {
                    id: "probe-call".into(),
                    name: "bash".into(),
                    input: json!({"command":"oc-resource-probe"}),
                }],
                usage: None,
            }])
            .with_default(vec![LlmEvent::Completed {
                text: "probe finished".into(),
                tool_calls: vec![],
                usage: None,
            }]),
    );
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    store
        .create_session(&SessionMeta {
            id: "probe-session".into(),
            agent: Some("probe".into()),
            title: Some("resource snapshot".into()),
            autopilot_mode: Some("off".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    let handles = opencoder_web::handle::new_handle_map();
    let state = Arc::new(opencoder_web::AppState {
        brain: opencoder_web::api_brain::mock_brain(store.clone()),
        store: store.clone(),
        workdir,
        handles: handles.clone(),
        nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
        controls: Arc::new(opencoder_web::control_state::ControlHub::new()),
        team: opencoder_web::team_state::mock(),
        project: opencoder_web::ProjectService::new(),
        client_override: Some(client.clone() as Arc<dyn ChatStream>),
    });
    let request = Request::builder()
        .method("POST")
        .uri("/api/sessions/probe-session/prompt")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"prompt":"$snapshot-probe run the resource probe"}).to_string(),
        ))
        .unwrap();
    let response = with_root(
        scoped.then_some(pinned),
        opencoder_web::build_app(state, None, false).oneshot(request),
    )
    .await
    .unwrap();
    assert!(response.status().is_success());
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let Some(handle) = handles.lock().await.get("probe-session").cloned() else {
                break;
            };
            if !handle.draining.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    if missing {
        assert!(client.requests().is_empty());
        let events = store.events_after("probe-session", 0).await.unwrap();
        assert!(events
            .iter()
            .any(|event| event.sse_kind.as_deref() == Some("error")
                && event.payload.to_string().contains("agent not found")));
        return;
    }
    let expected = if scoped { "PINNED" } else { "LIVE" };
    let requests = format!("{:?}", client.requests());
    assert!(
        requests.contains(&format!("{expected}_PROMPT")),
        "wrong prompt root"
    );
    assert!(
        requests.contains(&format!("{expected}_SKILL")),
        "wrong skill root"
    );
    let messages =
        serde_json::to_string(&store.load_messages("probe-session").await.unwrap()).unwrap();
    assert!(messages.contains(&format!("{expected}_TOOL")), "{messages}");
}

#[tokio::test]
async fn node_prompt_uses_pinned_prompt_skills_and_executable_tools() {
    check(true, false).await;
}

#[tokio::test]
async fn unscoped_web_prompt_keeps_configured_agent_resources() {
    check(false, false).await;
}

#[tokio::test]
async fn missing_node_snapshot_fails_without_using_live_resources() {
    check(true, true).await;
}
