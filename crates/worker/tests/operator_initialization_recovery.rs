#![cfg(unix)]
#[path = "../../session/tests/harness/binary.rs"]
mod binary;
mod support;

use opencoder_core::{fleet::*, harness::HarnessRuntime};
use opencoder_node::fleet::NodeService;
use opencoder_store::{LibsqlStore, Store};
use serde_json::{json, Value};
use support::{assignment, mock, settled, worker};

async fn recover(seed: usize) {
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    let container = support::native::container::ContainerFixture::open(&root.join("node"));
    let agents = container.pool.parent().unwrap().join("agents");
    std::fs::create_dir_all(agents.join("codexops")).unwrap();
    std::fs::write(
        agents.join("codexops/meta.json"),
        json!({
            "name":"codexops", "harness":"codex", "harness_profile":"server-profile",
            "current":{"prompt":"codexops"}
        })
        .to_string(),
    )
    .unwrap();
    let prompts = agents.join("prompts/codexops");
    std::fs::create_dir_all(prompts.join("v1")).unwrap();
    std::fs::write(prompts.join("meta.json"), r#"{"current":1}"#).unwrap();
    std::fs::write(prompts.join("v1/soul.md"), "Registered operator.").unwrap();
    std::fs::create_dir_all(root.join("work")).unwrap();
    let mut config = opencoder_core::Config::default();
    container.configure(&mut config);
    config.agent.agents_dir = container.config.agent.agents_dir.clone();
    std::fs::write(
        root.join("work/opencoder.json"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    let executable = binary::fake_binary(root).join("codex");
    let capture = root.join("capture.jsonl");
    let settings = json!({"profiles":{"server-profile":{"revision":1,"settings":{
        "executable":executable, "model":"server-model", "envs":{"CAPTURE":capture}
    }}}});
    let client = mock();
    let first = worker(root, client.clone()).await;
    let id = "operator-recover";
    let mut task = assignment(
        &first,
        id,
        ExecutionKind::Operator,
        json!({"prompt":"","literal_mentions":true,"envs":{"EXAMPLE":"launch-value"}}),
        None,
    );
    task.request.target = Some("codexops".into());
    task.runtime = Some(Box::new(serde_json::from_value(settings).unwrap()));
    let accepted = first
        .handle(NodeOperation::Create { assignment: task })
        .await;
    assert_eq!(accepted.status, 200, "{accepted:?}");
    assert_eq!(settled(&first, id).await["execution"]["status"], "idle");
    first.shutdown().await.unwrap();
    drop(first);

    let home = root.join("node/operator").join(id).join("home");
    let workspace = root.join("node/operator").join(id).join("workspace");
    std::fs::write(workspace.join("notes.md"), "literal mention").unwrap();
    let journal = root.join("node/operator").join(id).join("execution.json");
    let mut record: Value = serde_json::from_slice(&std::fs::read(&journal).unwrap()).unwrap();
    record["assignment"]["request"]["input"]["prompt"] = json!("inspect @notes.md");
    record["assignment"]["index"]["status"] = json!("running");
    record["result"] = Value::Null;
    std::fs::write(&journal, serde_json::to_vec(&record).unwrap()).unwrap();
    let store = LibsqlStore::open(root.join("node/runtime.db"))
        .await
        .unwrap();
    if seed == 0 {
        store
            .conn()
            .await
            .unwrap()
            .execute("UPDATE sessions SET harness_runtime=NULL WHERE id=?", [id])
            .await
            .unwrap();
    } else {
        let mut runtime = store.harness_runtime(id).await.unwrap().unwrap();
        if seed == 1 {
            runtime = HarnessRuntime {
                harness: opencoder_core::harness::Harness::Codex,
                envs: std::collections::BTreeMap::from([
                    ("EXAMPLE".into(), "launch-value".into()),
                    ("HOME".into(), home.to_string_lossy().into_owned()),
                ]),
                ..Default::default()
            };
        }
        runtime.literal_mentions = false;
        store.set_harness_runtime(id, &runtime).await.unwrap();
    }
    drop(store);
    // Node settings change after admission; the operator's frozen Server
    // profile and private HOME must still win on recovery.
    config.agent.codex = Some(opencoder_core::harness::CodexSettings {
        model: Some("changed-node-model".into()),
        ..Default::default()
    });
    std::fs::write(
        root.join("work/opencoder.json"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    let second = worker(root, client.clone()).await;
    let execution = ExecutionRef {
        id: id.into(),
        kind: ExecutionKind::Operator,
    };
    let reply = second
        .handle(NodeOperation::Command {
            execution: execution.clone(),
            command: ExecutionCommand {
                action: "resume".into(),
                input: json!({}),
            },
        })
        .await;
    assert_eq!(reply.status, 200, "{reply:?}");
    let detail = settled(&second, id).await;
    assert_eq!(detail["execution"]["status"], "idle", "{detail}");
    let reply = second
        .handle(NodeOperation::Command {
            execution,
            command: ExecutionCommand {
                action: "prompt".into(),
                input: json!({"prompt":"follow up @notes.md", "input_id":"follow-up"}),
            },
        })
        .await;
    assert_eq!(reply.status, 200, "{reply:?}");
    assert_eq!(settled(&second, id).await["execution"]["status"], "idle");
    assert_eq!(client.call_count(), 0, "native provider must remain unused");
    second.shutdown().await.unwrap();
    let records: Vec<Value> = std::fs::read_to_string(capture)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(records.len(), 2, "first prompt must execute once");
    for run in &records {
        assert_eq!(run["env"], "launch-value");
        assert_eq!(run["home"], home.to_string_lossy().as_ref());
        assert_eq!(run["cwd"], workspace.to_string_lossy().as_ref());
        assert!(run["args"]
            .as_array()
            .unwrap()
            .iter()
            .any(|arg| arg == "server-model"));
    }
    assert!(records[0]["prompt"]
        .as_str()
        .unwrap()
        .contains("inspect @notes.md"));
    assert_eq!(records[1]["prompt"], "follow up @notes.md");
    assert_eq!(records[1]["args"][1], "resume");
    let store = LibsqlStore::open(root.join("node/runtime.db"))
        .await
        .unwrap();
    assert!(
        store
            .harness_runtime(id)
            .await
            .unwrap()
            .unwrap()
            .literal_mentions
    );
    let messages = store.load_messages(id).await.unwrap();
    assert_eq!(
        messages
            .iter()
            .find(|message| message.role == opencoder_core::Role::User)
            .unwrap()
            .display
            .as_deref(),
        Some("inspect @notes.md")
    );
}

#[tokio::test]
async fn operator_recovery_completes_each_initialization_stage_and_keeps_server_profile() {
    let _isolation = support::isolated_config();
    for seed in 0..3 {
        recover(seed).await;
    }
}
