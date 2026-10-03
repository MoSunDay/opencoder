#![cfg(windows)]
use opencoder_core::fleet::*;
use opencoder_llm::{CompletedToolCall, LlmEvent, MockChatClient};
use opencoder_node::fleet::NodeService;
use opencoder_worker::{Worker, WorkerOptions};
use serde_json::{json, Value};
use std::{path::Path, sync::Arc, time::Duration};

fn assignment(node: &Worker, id: &str, kind: ExecutionKind, prompt: &str) -> Assignment {
    let node_id = node.registration().id;
    Assignment {
        private_context: None,
        runtime: None,
        codex: None,
        definition: None,
        index: ExecutionIndex {
            id: id.into(),
            kind,
            node_id: node_id.clone(),
            created_at: 1,
            status: ExecutionStatus::Pending,
        },
        request: CreateExecution {
            id: id.into(),
            kind,
            target: Some("act".into()),
            input: json!({"prompt":prompt}),
            node_id: Some(node_id),
        },
    }
}
async fn open(root: &Path, client: Arc<MockChatClient>) -> Worker {
    Worker::open(
        WorkerOptions {
            name: "windows-native".into(),
            workdir: root.join("work"),
            data_dir: root.join("node"),
            workflow_root: None,
            max_runs: Some(2),
            dag: true,
        },
        Some(client),
    )
    .await
    .unwrap()
}
async fn settled(node: &Worker, id: &str) -> Value {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let result = node
                .handle(NodeOperation::Inspect {
                    execution: ExecutionRef {
                        id: id.into(),
                        kind: ExecutionKind::Operator,
                    },
                })
                .await;
            assert_eq!(result.status, 200, "{result:?}");
            if result.body["execution"]["status"] == "idle" {
                break result.body;
            }
            assert_ne!(result.body["execution"]["status"], "failed", "{result:?}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn windows_operator_rejects_other_workloads_executes_and_recovers_isolated_home() {
    let root = tempfile::tempdir().unwrap();
    let _home = opencoder_core::scoped_config_home(root.path().join("config-home"));
    let executable = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("opencoder-agent.exe");
    opencoder_session::process::configure_supervisor_binary(executable).unwrap();
    std::fs::create_dir_all(root.path().join("work/.opencoder")).unwrap();
    std::fs::write(
        root.path().join("work/.opencoder/ap.json"),
        r#"{"mode":"off"}"#,
    )
    .unwrap();
    let call = || {
        vec![LlmEvent::Completed {
            text: String::new(),
            usage: None,
            tool_calls: vec![CompletedToolCall {
                id: "env".into(),
                name: "powershell".into(),
                input: json!({"command":"@{home=$env:HOME; profile=$env:USERPROFILE; roaming=$env:APPDATA; local=$env:LOCALAPPDATA; cwd=(Get-Location).Path} | ConvertTo-Json | Set-Content env.json"}),
            }],
        }]
    };
    let client = Arc::new(MockChatClient::new().push_script(call()).with_default(vec![
        LlmEvent::Completed {
            text: "native answer".into(),
            tool_calls: vec![],
            usage: None,
        },
    ]));
    let node = open(root.path(), client.clone()).await;
    assert_eq!(node.registration().kinds, vec![ExecutionKind::Operator]);
    for kind in [
        ExecutionKind::Dag,
        ExecutionKind::Brain,
        ExecutionKind::Agent,
        ExecutionKind::Team,
        ExecutionKind::Todos,
        ExecutionKind::Project,
        ExecutionKind::Maintenance,
    ] {
        let reply = node
            .handle(NodeOperation::Create {
                assignment: assignment(
                    &node,
                    &format!("{}-rejected", kind.prefix()),
                    kind,
                    "blocked",
                ),
            })
            .await;
        assert_eq!(reply.status, 400, "{reply:?}");
        assert_eq!(
            reply.body["error"],
            "Windows nodes only accept operator executions"
        );
    }
    let reply = node
        .handle(NodeOperation::Create {
            assignment: assignment(
                &node,
                "operator-native",
                ExecutionKind::Operator,
                "inspect environment",
            ),
        })
        .await;
    assert_eq!(reply.status, 200, "{reply:?}");
    settled(&node, "operator-native").await;
    let home = root
        .path()
        .join("node/operator/operator-native/home")
        .canonicalize()
        .unwrap();
    let workspace = root
        .path()
        .join("node/operator/operator-native/workspace")
        .canonicalize()
        .unwrap();
    let environment: Value =
        serde_json::from_slice(&std::fs::read(workspace.join("env.json")).unwrap()).unwrap();
    assert_eq!(Path::new(environment["home"].as_str().unwrap()), home);
    assert_eq!(environment["profile"], environment["home"]);
    assert_eq!(
        Path::new(environment["cwd"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        workspace
    );
    for key in ["roaming", "local"] {
        assert!(Path::new(environment[key].as_str().unwrap()).starts_with(&home));
    }
    assert!(
        opencoder_core::platform::fs::private_access(&home.join(".opencoder/config.json")).unwrap()
    );
    node.shutdown().await.unwrap();
    drop(node);
    let node = open(root.path(), client).await;
    let reply = node
        .handle(NodeOperation::Command {
            execution: ExecutionRef {
                id: "operator-native".into(),
                kind: ExecutionKind::Operator,
            },
            command: ExecutionCommand {
                action: "prompt".into(),
                input: json!({"prompt":"resume after restart"}),
            },
        })
        .await;
    assert_eq!(reply.status, 200, "{reply:?}");
    settled(&node, "operator-native").await;
    node.shutdown().await.unwrap();
}
