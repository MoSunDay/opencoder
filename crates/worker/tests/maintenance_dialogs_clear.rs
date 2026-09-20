//! `dialogs_clear` maintenance command: terminal operator sessions are deleted
//! from the node store together with their journal records (so the next full
//! index report cannot resurrect them), while a still-running execution is
//! reported back as skipped and keeps its data.
mod support;

use opencoder_core::fleet::*;
use opencoder_llm::{ChatRequest, ChatStream, LlmEvent};
use opencoder_node::fleet::NodeService;
use opencoder_store::{LibsqlStore, SessionFilter, Store};
use opencoder_worker::Worker;
use serde_json::json;
use std::sync::Arc;
use support::*;

/// A chat client that never answers until released, keeping an execution in
/// the running state while the maintenance call is exercised.
struct Hold {
    release: Arc<tokio::sync::Notify>,
}

impl ChatStream for Hold {
    fn chat_stream(
        &self,
        _request: ChatRequest,
    ) -> anyhow::Result<tokio::sync::mpsc::Receiver<LlmEvent>> {
        let release = self.release.clone();
        let (tx, rx) = tokio::sync::mpsc::channel(2);
        tokio::spawn(async move {
            release.notified().await;
            let _ = tx
                .send(LlmEvent::Completed {
                    text: "released".into(),
                    tool_calls: vec![],
                    usage: None,
                })
                .await;
        });
        Ok(rx)
    }
}

async fn run_operator(node: &Worker, id: &str) {
    run_session(node, id, ExecutionKind::Operator).await;
}

async fn run_session(node: &Worker, id: &str, kind: ExecutionKind) {
    let reply = node
        .handle(NodeOperation::Create {
            assignment: assignment(node, id, kind, json!({"prompt":"hi"}), None),
        })
        .await;
    assert_eq!(reply.status, 200, "{:?}", reply);
    let detail = settled(node, id).await;
    assert_eq!(detail["execution"]["status"], "idle", "{detail}");
}

#[tokio::test]
async fn dialogs_clear_deletes_sessions_and_journal_records() {
    let _config = isolated_config();
    let dir = tempfile::tempdir().unwrap();
    let node = worker(dir.path(), mock()).await;
    run_operator(&node, "operator-drop").await;
    run_operator(&node, "operator-keep").await;
    let drop_file = dir
        .path()
        .join("node/operator/operator-drop/execution.json");
    let keep_file = dir
        .path()
        .join("node/operator/operator-keep/execution.json");
    assert!(drop_file.is_file());
    assert!(keep_file.is_file());

    let reply = node
        .handle(NodeOperation::Maintenance {
            command: ExecutionCommand {
                action: "dialogs_clear".into(),
                input: json!({"sessions": ["operator-drop", "operator-ghost"]}),
            },
        })
        .await;
    assert_eq!(reply.status, 200, "{:?}", reply);
    assert_eq!(reply.body["removed"], json!(1));
    assert_eq!(reply.body["forgotten"], json!(1));
    assert_eq!(reply.body["skipped"], json!([]));

    // Journal record is gone, so the next index report cannot resurrect it.
    assert!(!drop_file.exists());
    assert!(keep_file.is_file());

    // Session rows follow the journal: drop deleted, keep untouched.
    node.shutdown().await.unwrap();
    let store = LibsqlStore::open(dir.path().join("node/runtime.db"))
        .await
        .unwrap();
    let sessions = store
        .list_sessions(&SessionFilter {
            limit: 100,
            ..Default::default()
        })
        .await
        .unwrap();
    let ids: Vec<String> = sessions.iter().map(|s| s.id.clone()).collect();
    assert!(!ids.contains(&"operator-drop".to_string()), "{ids:?}");
    assert!(ids.contains(&"operator-keep".to_string()), "{ids:?}");
}

#[tokio::test]
async fn dialogs_clear_spares_running_executions() {
    let _config = isolated_config();
    let dir = tempfile::tempdir().unwrap();
    let release = Arc::new(tokio::sync::Notify::new());
    let hold: Arc<dyn ChatStream> = Arc::new(Hold {
        release: release.clone(),
    });
    let node = worker(dir.path(), hold).await;
    let reply = node
        .handle(NodeOperation::Create {
            assignment: assignment(
                &node,
                "operator-live",
                ExecutionKind::Operator,
                json!({"prompt":"hi"}),
                None,
            ),
        })
        .await;
    assert_eq!(reply.status, 200, "{:?}", reply);
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let detail = node
                .handle(NodeOperation::Inspect {
                    execution: execution_ref(&node, "operator-live").await,
                })
                .await;
            if detail.body["execution"]["status"] == json!("running") {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();

    let reply = node
        .handle(NodeOperation::Maintenance {
            command: ExecutionCommand {
                action: "dialogs_clear".into(),
                input: json!({"sessions": ["operator-live"]}),
            },
        })
        .await;
    assert_eq!(reply.status, 200, "{:?}", reply);
    assert_eq!(reply.body["removed"], json!(0));
    assert_eq!(reply.body["skipped"], json!(["operator-live"]));
    assert!(dir
        .path()
        .join("node/operator/operator-live/execution.json")
        .is_file());

    release.notify_one();
    let detail = settled(&node, "operator-live").await;
    assert_eq!(detail["execution"]["status"], "idle", "{detail}");
    node.shutdown().await.unwrap();
}

#[tokio::test]
async fn dialogs_clear_rejects_cross_lane_ids_and_protects_internal_sessions() {
    let _config = isolated_config();
    let dir = tempfile::tempdir().unwrap();
    let node = worker(dir.path(), mock()).await;
    run_session(&node, "operator-lane", ExecutionKind::Operator).await;
    run_session(&node, "agent-lane", ExecutionKind::Agent).await;

    let mismatch = node
        .handle(NodeOperation::Maintenance {
            command: ExecutionCommand {
                action: "dialogs_clear".into(),
                input: json!({"kind": "agent", "sessions": ["operator-lane"]}),
            },
        })
        .await;
    assert_eq!(mismatch.status, 409, "{mismatch:?}");
    assert!(dir
        .path()
        .join("node/operator/operator-lane/execution.json")
        .is_file());

    let store = LibsqlStore::open(dir.path().join("node/runtime.db"))
        .await
        .unwrap();
    store
        .create_session(&opencoder_store::SessionMeta {
            id: "internal-child".into(),
            title: Some("child".into()),
            agent: Some("act".into()),
            task_type: Some("subagent".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    let internal = node
        .handle(NodeOperation::Maintenance {
            command: ExecutionCommand {
                action: "dialogs_clear".into(),
                input: json!({"kind": "agent", "sessions": ["internal-child"]}),
            },
        })
        .await;
    assert_eq!(internal.status, 200, "{internal:?}");
    assert_eq!(internal.body["removed"], json!(0));
    assert!(store.get_session("internal-child").await.unwrap().is_some());

    let cleared = node
        .handle(NodeOperation::Maintenance {
            command: ExecutionCommand {
                action: "dialogs_clear".into(),
                input: json!({"kind": "agent", "sessions": ["agent-lane"]}),
            },
        })
        .await;
    assert_eq!(cleared.status, 200, "{cleared:?}");
    assert_eq!(cleared.body["kind"], json!("agent"));
    assert_eq!(cleared.body["removed"], json!(1));

    let repeat = node
        .handle(NodeOperation::Maintenance {
            command: ExecutionCommand {
                action: "dialogs_clear".into(),
                input: json!({"kind": "agent", "sessions": ["agent-lane"]}),
            },
        })
        .await;
    assert_eq!(repeat.status, 200, "{repeat:?}");
    assert_eq!(repeat.body["removed"], json!(0));
    node.shutdown().await.unwrap();
}
