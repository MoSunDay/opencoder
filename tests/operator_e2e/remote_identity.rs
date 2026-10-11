//! Both conversation kinds execute a Server-managed identity on the remote host.
use crate::support::fleet_proc::{Fleet, TOKEN};
use crate::support::http_util::sse_read;
use crate::support::llm_stub::{LlmStub, Script};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};

fn file(path: &str, content: &str, mode: u32) -> Value {
    json!({"path":path,"content_b64":STANDARD.encode(content),"mode":mode})
}

fn register(fleet: &Fleet) {
    let (status, body) = fleet.http(
        "POST",
        "/api/agents",
        &json!({
            "name":"remote-identity", "harness":"opencoder"
        }),
    );
    assert_eq!(status, 201, "{body}");
    for (category, files) in [
        ("prompts", vec![
            file("soul.md", "# Remote identity\nSERVER_SOUL_PROOF\n", 0o644),
            file("how.md", "SERVER_HOW_PROOF\n", 0o644),
        ]),
        ("skills", vec![file("identity-proof/SKILL.md",
            "---\nname: identity-proof\ndescription: Verify remote identity\n---\nSERVER_SKILL_PROOF\n", 0o644)]),
        ("tools", vec![file("identity-proof",
            "#!/bin/sh\nprintf 'SERVER_TOOL_PROOF\\n'\nprintf 'REMOTE_WORKSPACE_PROOF\\n' > remote-proof.txt\n", 0o755)]),
    ] {
        let path = format!("/api/agents/remote-identity/resources/{category}");
        let (status, view) = fleet.http("GET", &path, &Value::Null);
        assert_eq!(status, 200, "{view}");
        let (status, body) = fleet.http("PUT", &path,
            &json!({"baseline":view["baseline"],"files":files}));
        assert_eq!(status, 200, "{category}: {body}");
    }
}

fn tool() -> Script {
    Script::ToolCall {
        name: "bash".into(),
        arguments: json!({"command":"identity-proof"}).to_string(),
    }
}

fn run_identity(kind: &str) {
    let stub = LlmStub::spawn(vec![
        tool(),
        Script::Text("REMOTE_FIRST_REPLY".into()),
        tool(),
        Script::Text("REMOTE_SECOND_REPLY".into()),
    ]);
    let root = tempfile::tempdir().unwrap();
    let mut fleet = Fleet::spawn_conversations(root.path(), stub.port(), "remote-identity-node");
    register(&fleet);
    // Large installations retain unused resource pools after Agent removal.
    // Their contents must not participate in a new conversation's admission.
    let orphan = root.path().join("native-source/agents/tools/unreferenced");
    std::fs::create_dir_all(&orphan).unwrap();
    std::fs::write(orphan.join("meta.json"), "invalid unused resource").unwrap();
    let id = format!("{kind}-remote-identity");
    // Keep automatic title generation out of the scripted conversation calls.
    let input = json!({"id":id,"kind":kind,"agent":"remote-identity","title":"Remote identity",
        "node_id":fleet.node_id(),"prompt":"$identity-proof verify the remote identity"});
    assert!(
        !fleet.node_data.join("dag/rootfs").exists(),
        "ordinary sessions need no container image"
    );
    let (status, body) = fleet.http("POST", "/api/sessions", &input);
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["id"], id);
    let pinned = fleet.node_data.join(kind).join(&id).join("resources");
    assert!(pinned.join("remote-identity/meta.json").is_file());
    assert!(!pinned.join("tools/unreferenced").exists());
    let frames = sse_read(
        &fleet.base,
        &format!("/api/sessions/{id}/events"),
        TOKEN,
        0,
        None,
    );
    for event in ["text_delta", "tool_start", "tool_end", "done"] {
        assert!(
            frames.iter().any(|frame| frame.event == event),
            "missing {event}"
        );
    }
    let result = fleet.wait_idle(&id);
    assert_eq!(
        result["result"]["output_text"], "REMOTE_FIRST_REPLY",
        "{result}"
    );
    let requests = stub.wait_for_requests(2);
    for marker in [
        "SERVER_SOUL_PROOF",
        "SERVER_HOW_PROOF",
        "SERVER_SKILL_PROOF",
    ] {
        assert!(
            requests[0].contains(marker),
            "missing {marker} in model request"
        );
    }
    assert!(
        requests[1].contains("SERVER_TOOL_PROOF"),
        "remote tool must execute: {:?}",
        serde_json::from_str::<Value>(&requests[1]).unwrap()["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|message| message["role"] == "tool")
            .collect::<Vec<_>>()
    );
    let workspace = if kind == "operator" {
        fleet.node_data.join("operator").join(&id).join("workspace")
    } else {
        fleet.workdir.join("node-work")
    };
    assert_eq!(
        std::fs::read_to_string(workspace.join("remote-proof.txt"))
            .unwrap()
            .trim(),
        "REMOTE_WORKSPACE_PROOF"
    );
    assert!(
        !fleet.workdir.join("remote-proof.txt").exists(),
        "Server must not execute tools"
    );
    // A retried creation must not invoke the model twice.
    assert_eq!(fleet.http("POST", "/api/sessions", &input).0, 200);
    assert_eq!(stub.request_count(), 2);
    fleet.respawn_agent();
    let (status, body) = fleet.http(
        "POST",
        &format!("/api/sessions/{id}/prompt"),
        &json!({"input_id":"follow-up","prompt":"$identity-proof verify the same identity again"}),
    );
    assert!(status < 300, "{body}");
    let result = fleet.wait_idle(&id);
    assert_eq!(
        result["result"]["output_text"], "REMOTE_SECOND_REPLY",
        "{result}"
    );
    let (_, snapshot) = fleet.http("GET", &format!("/api/sessions/{id}"), &Value::Null);
    assert_eq!(snapshot["meta"]["agent"], "remote-identity");
    for text in [
        "REMOTE_FIRST_REPLY",
        "REMOTE_SECOND_REPLY",
        "SERVER_TOOL_PROOF",
    ] {
        assert!(
            snapshot["messages"].to_string().contains(text),
            "{snapshot}"
        );
    }
    let (_, indexes) = fleet.http("GET", "/api/executions", &Value::Null);
    assert_eq!(indexes["executions"].as_array().unwrap().len(), 1);
    assert!(!fleet.node_data.join("dag/rootfs").exists());
}

#[test]
fn operator_uses_server_identity_skills_tools_and_resumes_on_host() {
    run_identity("operator");
}

#[test]
fn agent_uses_server_identity_skills_tools_and_resumes_on_host() {
    run_identity("agent");
}

#[test]
fn missing_identity_is_rejected_without_a_server_execution_record() {
    let stub = LlmStub::spawn_text(&["must not execute"]);
    let root = tempfile::tempdir().unwrap();
    let fleet = Fleet::spawn(root.path(), stub.port(), "missing-identity-node");
    for kind in ["operator", "agent"] {
        let id = format!("{kind}-missing-identity");
        let (status, body) = fleet.http(
            "POST",
            "/api/sessions",
            &json!({
                "id":id,"kind":kind,"agent":"missing-identity","prompt":"do not fall back"
            }),
        );
        assert_eq!(status, 400, "{body}");
        assert!(body["error"].as_str().unwrap().contains("missing-identity"));
        assert_eq!(
            fleet
                .http("GET", &format!("/api/executions/{id}/index"), &Value::Null)
                .0,
            404
        );
    }
    assert!(stub.request_count() == 0);
}
