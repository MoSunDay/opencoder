#![cfg(unix)]
#[path = "../../session/tests/harness/binary.rs"]
mod binary;
mod support;
use serde_json::{json, Value};
use support::*;

#[tokio::test]
async fn registered_codex_operator_uses_server_settings_auto_placement_and_same_thread() {
    let client = mock();
    let fleet = Fleet::new_with_agents(1, client.clone(), |agents| {
        std::fs::create_dir_all(agents.join("codexops")).unwrap();
        std::fs::write(
            agents.join("codexops/meta.json"),
            json!({"name":"codexops","harness":"codex","current":{"prompt":"codexops"}})
                .to_string(),
        )
        .unwrap();
        let prompt = agents.join("prompts/codexops");
        std::fs::create_dir_all(prompt.join("v1")).unwrap();
        std::fs::write(prompt.join("meta.json"), r#"{"current":1}"#).unwrap();
        std::fs::write(
            prompt.join("v1/soul.md"),
            "Use the registered Codex wrapper.",
        )
        .unwrap();
    })
    .await;
    fleet.state.fleet.put_definition("brain_capability", "codex-ops", &json!({"id":"codex-ops","kind":"operator","target":"codexops","summary":"Registered Codex"})).await.unwrap();
    let catalog = fleet
        .call("GET", "/api/tui/agent-capabilities", Value::Null)
        .await;
    assert_eq!(catalog.status, 200, "{catalog:?}");
    let capability = catalog.body["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|card| card["id"] == "codex-ops")
        .unwrap();
    assert_eq!(capability["kind"], "operator");
    assert_eq!(capability["target"], "codexops");
    let executable = binary::fake_binary(fleet.root()).join("codex");
    let capture = fleet.root().join("operator.jsonl");
    let configured = fleet
        .call(
            "PUT",
            "/api/harnesses/codex",
            json!({"executable":executable,"envs":{"CAPTURE":capture},"model":"server-codex"}),
        )
        .await;
    assert_eq!(configured.status, 200, "{configured:?}");
    let request = json!({"id":"operator-tui-codex","kind":capability["kind"],"target":capability["target"],"input":{"prompt":"inspect @notes.md","literal_mentions":true}});
    let receipt = fleet.call("POST", "/api/executions", request.clone()).await;
    assert_eq!(receipt.status, 202, "{receipt:?}");
    let first = settled(&fleet.nodes[0], "operator-tui-codex").await;
    assert_eq!(first["session"]["harness"], "codex", "{first}");
    assert_eq!(first["execution"]["status"], "idle");
    let again = fleet.call("POST", "/api/executions", request).await;
    assert_eq!(again.status, 202);
    let continued=fleet.call("POST","/api/executions/operator-tui-codex/commands",json!({"action":"prompt","input":{"prompt":"follow up @notes.md","input_id":"follow-up"}})).await;
    assert!(continued.status < 300, "{continued:?}");
    let second = settled(&fleet.nodes[0], "operator-tui-codex").await;
    assert_eq!(second["execution"]["status"], "idle", "{second}");
    assert_eq!(
        client.call_count(),
        0,
        "Operator must use its registered Codex wrapper"
    );
    let runs: Vec<Value> = std::fs::read_to_string(&capture)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(runs.len(), 2);
    assert!(
        runs.iter().all(|run| run["args"]
            .as_array()
            .unwrap()
            .iter()
            .any(|arg| arg == "server-codex")),
        "Codex must use the model configured by Server: {runs:?}"
    );
    assert!(runs[0]["prompt"]
        .as_str()
        .unwrap()
        .contains("inspect @notes.md"));
    assert_eq!(runs[1]["prompt"], "follow up @notes.md");
    assert_eq!(runs[1]["args"][1], "resume");
    let events = fleet
        .call(
            "GET",
            "/api/executions/operator-tui-codex/events-page",
            Value::Null,
        )
        .await;
    assert_eq!(events.status, 200);
    let events = events.body["events"].as_array().unwrap();
    for kind in ["text_delta", "tool_start", "tool_end", "done"] {
        assert!(
            events.iter().any(|event| event["kind"] == kind),
            "missing {kind}: {events:?}"
        );
    }
    let page = fleet
        .call(
            "POST",
            "/api/executions/operator-tui-codex/commands",
            json!({"action":"http","input":{"method":"GET","tail":"transcript?seq=0&offset=0"}}),
        )
        .await;
    assert_eq!(page.status, 200, "{page:?}");
    let first_chunk = &page.body["chunks"][0];
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(first_chunk["bytes_b64"].as_str().unwrap())
        .unwrap();
    let message: opencoder_core::Message = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(message.display.as_deref(), Some("inspect @notes.md"));
}
