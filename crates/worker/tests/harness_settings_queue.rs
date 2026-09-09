#![cfg(unix)]
#[path = "../../session/tests/harness/binary.rs"]
mod binary;
mod support;
use opencoder_core::fleet::*;
use opencoder_node::fleet::NodeService;
use serde_json::{json, Value};
use std::{path::Path, time::Duration};
use support::*;

fn controlled_binary(root: &Path) -> std::path::PathBuf {
    let path = binary::fake_binary(root).join("codex");
    let source = std::fs::read_to_string(&path).unwrap();
    let source = source.replace("def emit(v):", "import pathlib, re\nlabel = re.findall(r'TASK:(\\w+)', prompt)[-1]\nwhile label.startswith('hold') and not pathlib.Path(os.environ['GATES'], label).exists(): time.sleep(0.02)\ndef emit(v):");
    std::fs::write(&path, source).unwrap();
    path
}

fn capture(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

async fn wait_count(path: &Path, count: usize) {
    tokio::time::timeout(Duration::from_secs(15), async {
        while capture(path).len() < count {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("Codex start count");
}

fn request(id: &str, label: &str) -> Value {
    json!({"id":id,"kind":"agent","target":"act","input":{"harness":"codex","prompt":format!("TASK:{label}")}})
}

async fn managed(fleet: &Fleet) -> (Value, std::path::PathBuf) {
    let binary = controlled_binary(fleet.root());
    let log = fleet.root().join("managed-capture.jsonl");
    let config = json!({"executable":binary,"model":"codex-managed-model","reasoning_effort":"high", "sandbox_mode":"read-only", "approval_policy":"never",
        "envs":{"CAPTURE":log,"GATES":fleet.root(),"EXAMPLE":"private-managed-value"}});
    let saved = fleet
        .call("PUT", "/api/harnesses/codex", config.clone())
        .await;
    assert_eq!(saved.status, 200, "{saved:?}");
    assert_eq!(saved.body["revision"], 1);
    (config, log)
}

#[tokio::test]
async fn managed_codex_is_pinned_and_node_obeys_fifo_lifo() {
    for (order, expected) in [("fifo", ["autoA", "autoB"]), ("lifo", ["autoB", "autoA"])] {
        let client = mock();
        let fleet = Fleet::new(1, client.clone()).await;
        let node_id = fleet.nodes[0].registration().id;
        let settings_path = format!("/api/nodes/{node_id}/scheduling");
        let updated = fleet
            .call(
                "PUT",
                &settings_path,
                json!({"max_runs":1,"queue_order":order}),
            )
            .await;
        assert_eq!(updated.status, 200, "{updated:?}");
        let (mut config, log) = managed(&fleet).await;
        let first = fleet
            .call(
                "POST",
                "/api/executions",
                request("agent-hold", "holdFirst"),
            )
            .await;
        assert_eq!(first.status, 202, "{first:?}");
        wait_count(&log, 1).await;
        for (id, label) in [("agent-a", "autoA"), ("agent-b", "autoB")] {
            let body = request(id, label);
            let result = fleet.call("POST", "/api/executions", body.clone()).await;
            assert_eq!(result.status, 202, "{result:?}");
            assert_eq!(result.body["status"], "pending");
            let retry = fleet.call("POST", "/api/executions", body).await;
            assert_eq!(retry.status, 202, "{retry:?}");
            assert_eq!(retry.body["status"], "pending");
        }
        let denied = fleet.call("POST", "/api/executions", json!({"id":"agent-override","kind":"agent","target":"act","input":{"harness":"codex","prompt":"TASK:autoInvalid","envs":{"EXAMPLE":"override"}}})).await;
        assert_eq!(denied.status, 400, "{denied:?}");
        config["envs"]["EXAMPLE"] = json!("new-managed-value");
        assert_eq!(
            fleet
                .call("PUT", "/api/harnesses/codex", config)
                .await
                .status,
            200
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(
            capture(&log).len(),
            1,
            "full node must not start queued work"
        );
        std::fs::write(fleet.root().join("holdFirst"), "release").unwrap();
        wait_count(&log, 3).await;
        for id in ["agent-hold", "agent-a", "agent-b"] {
            let detail = settled(&fleet.nodes[0], id).await;
            assert_eq!(detail["execution"]["status"], "idle", "{detail}");
            assert!(!detail.to_string().contains("private-managed-value"));
        }
        let rows = capture(&log);
        for (row, label) in rows[1..].iter().zip(expected) {
            assert!(
                row["prompt"]
                    .as_str()
                    .unwrap()
                    .contains(&format!("TASK:{label}")),
                "{row}"
            );
            assert_eq!(row["env"], "private-managed-value");
            let args = row["args"].as_array().unwrap();
            assert!(args.contains(&json!("codex-managed-model")));
            assert!(args.contains(&json!("model_reasoning_effort=\"high\"")));
            assert!(args.contains(&json!("sandbox_mode=\"read-only\"")));
        }
        assert_eq!(client.call_count(), 0);
        let invalid = fleet
            .call(
                "PUT",
                &settings_path,
                json!({"max_runs":0,"queue_order":"fifo"}),
            )
            .await;
        assert_eq!(invalid.status, 400);
        assert_eq!(fleet.nodes[0].snapshot().max_runs, 1);
        fleet.shutdown().await;
    }
}

#[tokio::test]
async fn pending_queue_and_scheduling_survive_node_restart() {
    let root = tempfile::tempdir().unwrap();
    let binary = controlled_binary(root.path());
    let log = root.path().join("restart.jsonl");
    let settings: opencoder_core::harness::CodexSettings = serde_json::from_value(json!({"executable":binary,"envs":{"CAPTURE":log,"GATES":root.path(),"EXAMPLE":"restart-value"}})).unwrap();
    let node = worker(root.path(), mock()).await;
    let reply = node
        .handle(NodeOperation::Maintenance {
            command: ExecutionCommand {
                action: "configure_scheduling".into(),
                input: json!({"max_runs":1,"queue_order":"lifo"}),
            },
        })
        .await;
    assert_eq!(reply.status, 200);
    for (id, label) in [
        ("agent-first", "holdRestart"),
        ("agent-second", "autoSecond"),
        ("agent-third", "autoThird"),
    ] {
        let mut assigned = assignment(
            &node,
            id,
            ExecutionKind::Agent,
            request(id, label)["input"].clone(),
            None,
        );
        assigned.codex = Some(Box::new(settings.clone()));
        let accepted = node
            .handle(NodeOperation::Create {
                assignment: assigned,
            })
            .await;
        assert_eq!(accepted.status, 200, "{accepted:?}");
        if id == "agent-first" {
            wait_count(&log, 1).await;
        } else {
            assert_eq!(accepted.body["status"], "pending");
        }
    }
    node.shutdown().await.unwrap();
    drop(node);
    tokio::time::sleep(Duration::from_millis(150)).await;
    let restarted = worker(root.path(), mock()).await;
    assert_eq!(
        restarted.snapshot().max_runs,
        1,
        "persisted setting wins over startup default 4"
    );
    assert_eq!(restarted.snapshot().queue_order, QueueOrder::Lifo);
    wait_count(&log, 3).await;
    for id in ["agent-second", "agent-third"] {
        assert_eq!(settled(&restarted, id).await["execution"]["status"], "idle");
    }
    let rows = capture(&log);
    assert!(rows[1]["prompt"]
        .as_str()
        .unwrap()
        .contains("TASK:autoThird"));
    assert!(rows[2]["prompt"]
        .as_str()
        .unwrap()
        .contains("TASK:autoSecond"));
    assert_eq!(rows.len(), 3);
    restarted.shutdown().await.unwrap();
}

#[tokio::test]
async fn dynamic_limit_does_not_interrupt_active_work_and_pending_cancel_never_starts() {
    let fleet = Fleet::new(1, mock()).await;
    let path = format!("/api/nodes/{}/scheduling", fleet.nodes[0].registration().id);
    fleet.call("PUT", &path, json!({"max_runs":1})).await;
    let (_, log) = managed(&fleet).await;
    fleet
        .call("POST", "/api/executions", request("agent-one", "holdOne"))
        .await;
    wait_count(&log, 1).await;
    assert_eq!(
        fleet
            .call("POST", "/api/executions", request("agent-two", "holdTwo"))
            .await
            .body["status"],
        "pending"
    );
    assert_eq!(
        fleet.call("PUT", &path, json!({"max_runs":2})).await.status,
        200
    );
    wait_count(&log, 2).await;
    assert_eq!(
        fleet.call("PUT", &path, json!({"max_runs":1})).await.status,
        200
    );
    assert_eq!(fleet.nodes[0].snapshot().active_runs, 2);
    for (id, label) in [("agent-cancel", "autoCancel"), ("agent-three", "autoThree")] {
        assert_eq!(
            fleet
                .call("POST", "/api/executions", request(id, label))
                .await
                .body["status"],
            "pending"
        );
    }
    let cancelled = fleet
        .call(
            "POST",
            "/api/executions/agent-cancel/commands",
            json!({"action":"cancel"}),
        )
        .await;
    assert_eq!(cancelled.body["status"], "cancelled");
    std::fs::write(fleet.root().join("holdOne"), "release").unwrap();
    settled(&fleet.nodes[0], "agent-one").await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        capture(&log).len(),
        2,
        "lowered limit must wait until enough active work finishes"
    );
    std::fs::write(fleet.root().join("holdTwo"), "release").unwrap();
    wait_count(&log, 3).await;
    settled(&fleet.nodes[0], "agent-three").await;
    assert!(capture(&log)[2]["prompt"]
        .as_str()
        .unwrap()
        .contains("TASK:autoThree"));
    assert_eq!(capture(&log).len(), 3);
    fleet.shutdown().await;
}

#[tokio::test]
async fn idle_codex_followup_waits_for_capacity_and_keeps_its_settings() {
    let fleet = Fleet::new(1, mock()).await;
    let path = format!("/api/nodes/{}/scheduling", fleet.nodes[0].registration().id);
    assert_eq!(
        fleet.call("PUT", &path, json!({"max_runs":1})).await.status,
        200
    );
    let (mut config, log) = managed(&fleet).await;
    assert_eq!(
        fleet
            .call(
                "POST",
                "/api/executions",
                request("agent-chat", "autoInitial")
            )
            .await
            .status,
        202
    );
    settled(&fleet.nodes[0], "agent-chat").await;
    assert_eq!(
        fleet
            .call(
                "POST",
                "/api/executions",
                request("agent-block", "holdBlock")
            )
            .await
            .status,
        202
    );
    wait_count(&log, 2).await;
    config["envs"]["EXAMPLE"] = json!("updated-value");
    fleet.call("PUT", "/api/harnesses/codex", config).await;
    let command = json!({"action":"prompt", "input":{"input_id":"followup-stable", "prompt":"TASK:autoFollowup"}});
    for _ in 0..2 {
        let accepted = fleet
            .call(
                "POST",
                "/api/executions/agent-chat/commands",
                command.clone(),
            )
            .await;
        assert!(accepted.status < 300, "{accepted:?}");
        assert_eq!(accepted.body["status"], "pending");
    }
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(capture(&log).len(), 2);
    std::fs::write(fleet.root().join("holdBlock"), "release").unwrap();
    wait_count(&log, 3).await;
    let detail = settled(&fleet.nodes[0], "agent-chat").await;
    assert_eq!(detail["execution"]["status"], "idle", "{detail}");
    let rows = capture(&log);
    assert_eq!(rows.len(), 3);
    assert!(rows[2]["args"]
        .as_array()
        .unwrap()
        .contains(&json!("resume")));
    assert_eq!(rows[2]["env"], "private-managed-value");
    assert!(rows[2]["prompt"]
        .as_str()
        .unwrap()
        .contains("TASK:autoFollowup"));
    fleet.shutdown().await;
}
