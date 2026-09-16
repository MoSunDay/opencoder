#![cfg(unix)]
#[path = "../../session/tests/harness/binary.rs"]
mod binary;
mod support;
use opencoder_core::fleet::*;
use opencoder_node::fleet::NodeService;
use serde_json::json;
use std::time::Duration;
use support::*;

fn request(id: &str, label: &str) -> serde_json::Value {
    json!({"id":id,"kind":"agent","target":"act","input":{"harness":"codex","prompt":format!("TASK:{label}")}})
}

fn capture(path: &std::path::Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[tokio::test]
async fn scheduling_workdir_routes_node_sessions_to_the_configured_workspace() {
    let _config = isolated_config();
    let fleet = Fleet::new(1, mock()).await;
    let node_id = fleet.nodes[0].registration().id;
    let settings_path = format!("/api/nodes/{node_id}/scheduling");
    let workspace = fleet.root().join("workspace");

    // Relative workspaces are refused before the node is even contacted.
    let rejected = fleet
        .call(
            "PUT",
            &settings_path,
            json!({"max_runs":2,"queue_order":"fifo","workdir":"rel/dir"}),
        )
        .await;
    assert_eq!(rejected.status, 400, "{rejected:?}");

    let saved = fleet
        .call(
            "PUT",
            &settings_path,
            json!({"max_runs":2,"queue_order":"fifo","workdir":workspace.to_string_lossy()}),
        )
        .await;
    assert_eq!(saved.status, 200, "{saved:?}");
    assert_eq!(saved.body["workdir"], &*workspace.to_string_lossy());
    // The node provisions the workspace immediately, before any session runs.
    assert!(workspace.is_dir(), "node must create the workspace");

    let read_back = fleet
        .call("GET", &settings_path, serde_json::Value::Null)
        .await;
    assert_eq!(read_back.status, 200, "{read_back:?}");
    assert_eq!(read_back.body["workdir"], &*workspace.to_string_lossy());

    let binary = binary::fake_binary(fleet.root()).join("codex");
    let log = fleet.root().join("capture.jsonl");
    let config = json!({"executable":binary,"model":"codex-managed-model","reasoning_effort":"high","sandbox_mode":"read-only","approval_policy":"never","envs":{"CAPTURE":log}});
    let saved_harness = fleet.call("PUT", "/api/harnesses/codex", config).await;
    assert_eq!(saved_harness.status, 200, "{saved_harness:?}");

    let result = fleet
        .call("POST", "/api/executions", request("agent-ws", "wsCheck"))
        .await;
    assert_eq!(result.status, 202, "{result:?}");
    let detail = settled(&fleet.nodes[0], "agent-ws").await;
    assert_eq!(detail["execution"]["status"], "idle", "{detail:?}");

    let rows = capture(&log);
    assert_eq!(rows.len(), 1, "{rows:?}");
    let cwd = std::path::PathBuf::from(rows[0]["cwd"].as_str().unwrap());
    assert_eq!(
        cwd,
        workspace.canonicalize().unwrap(),
        "codex session must run inside the scheduling workspace"
    );

    // Clearing the field restores the node's startup workdir.
    let cleared = fleet
        .call(
            "PUT",
            &settings_path,
            json!({"max_runs":2,"queue_order":"fifo","workdir":serde_json::Value::Null}),
        )
        .await;
    assert_eq!(cleared.status, 200, "{cleared:?}");
    let read_back = fleet
        .call("GET", &settings_path, serde_json::Value::Null)
        .await;
    assert_eq!(read_back.status, 200, "{read_back:?}");
    assert!(read_back.body["workdir"].is_null(), "{read_back:?}");
    fleet.shutdown().await;
}

#[tokio::test]
async fn scheduling_workdir_survives_node_restart_and_stamps_sessions() {
    let _config = isolated_config();
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let node = worker(root.path(), mock()).await;
    let settings = NodeScheduling {
        max_runs: 2,
        queue_order: QueueOrder::Fifo,
        workdir: Some(workspace.to_string_lossy().into_owned()),
    };
    settings.validate().unwrap();
    let dir = root.path().join("node");
    std::fs::write(
        dir.join("scheduling.json"),
        serde_json::to_vec(&settings).unwrap(),
    )
    .unwrap();
    node.shutdown().await.unwrap();
    drop(node);
    tokio::time::sleep(Duration::from_millis(150)).await;

    let restarted = worker(root.path(), mock()).await;
    assert_eq!(
        restarted.snapshot().max_runs,
        2,
        "persisted scheduling wins over startup default"
    );
    // The workspace is provisioned again on boot without failing startup.
    assert!(workspace.is_dir());
    restarted.shutdown().await.unwrap();
}
