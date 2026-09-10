mod support;
use opencoder_core::fleet::*;
use opencoder_llm::{CompletedToolCall, LlmEvent, MockChatClient};
use opencoder_node::fleet::NodeService;
use serde_json::{json, Value};
use std::sync::Arc;
use support::*;
fn done(text: &str) -> Vec<LlmEvent> {
    vec![LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![],
        usage: None,
    }]
}

#[tokio::test]
async fn team_members_execute_locally_with_capability_prefixes() {
    let dir = tempfile::tempdir().unwrap();
    let client=Arc::new(MockChatClient::new().with_default(done(r#"{"question":"inspect","participants":["plan"],"summary":"aligned","aligned":true,"complete":true,"final_summary":"team completed"}"#)));
    let node = worker(dir.path(), client.clone()).await;
    let definition = json!({"name":"review","captain":"act","members":[{"agent":"act"},{"agent":"plan","capabilities":["review implementation"]}]});
    let reply = node
        .handle(NodeOperation::Create {
            assignment: assignment(
                &node,
                "team-local",
                ExecutionKind::Team,
                json!({"prompt":"review change"}),
                Some(definition),
            ),
        })
        .await;
    assert_eq!(reply.status, 200, "{:?}", reply);
    let detail = settled(&node, "team-local").await;
    assert_eq!(detail["execution"]["status"], "done", "{detail}");
    assert_eq!(detail["topic"]["final_summary"], "team completed");
    assert!(dir
        .path()
        .join("node/team/team-local/execution.json")
        .is_file());
    assert!(dir
        .path()
        .join("node/team/team-local/team/review/team.json")
        .is_file());
    let indexes = node.indexes().await.unwrap();
    assert!(
        indexes
            .iter()
            .filter(|i| i.id.starts_with("member-"))
            .count()
            >= 3
    );
    assert!(indexes.iter().all(|i| i.node_id == node.registration().id));
    assert!(client
        .requests()
        .iter()
        .any(|r| serde_json::to_string(&r.messages)
            .unwrap()
            .contains("review implementation")));
    node.shutdown().await.unwrap();
}

#[tokio::test]
async fn dag_artifacts_and_checkpoints_survive_node_restart() {
    let dir = tempfile::tempdir().unwrap();
    let client = mock();
    let node = worker(dir.path(), client.clone()).await;
    // Stage the wasm module into the run context root before execution.
    support::stage_stdout_wasm(&dir.path().join("node"), "tool.wasm", "artifact on node");
    let spec = json!({"name":"local-dag","steps":[{"name":"first","kind":{"type":"wasm","command":"tool.wasm"}},{"name":"review","depends_on":["first"],"kind":{"type":"agent","prompt":"review result"}}]});
    assert_eq!(
        node.handle(NodeOperation::Create {
            assignment: assignment(
                &node,
                "dag-checkpoint",
                ExecutionKind::Dag,
                json!({}),
                Some(spec)
            )
        })
        .await
        .status,
        200
    );
    let detail = settled(&node, "dag-checkpoint").await;
    assert_eq!(detail["execution"]["status"], "done", "{detail}");
    let path = dir.path().join("node/dag/dag-checkpoint/first/output.txt");
    assert!(std::fs::read_to_string(path)
        .unwrap()
        .contains("artifact on node"));
    node.shutdown().await.unwrap();
    drop(node);
    let path = dir.path().join("node/dag/dag-checkpoint/execution.json");
    let mut record: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    record["assignment"]["index"]["status"] = json!("running");
    std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    let calls = client.call_count();
    let node = worker(dir.path(), client.clone()).await;
    let reply = node
        .handle(NodeOperation::Command {
            execution: ExecutionRef {
                id: "dag-checkpoint".into(),
                kind: ExecutionKind::Dag,
            },
            command: ExecutionCommand {
                action: "resume".into(),
                input: json!({}),
            },
        })
        .await;
    assert_eq!(reply.status, 200, "{:?}", reply);
    let detail = settled(&node, "dag-checkpoint").await;
    assert_eq!(detail["execution"]["status"], "done", "{detail}");
    assert_eq!(
        client.call_count(),
        calls,
        "completed step checkpoint must not rerun"
    );
    node.shutdown().await.unwrap();
}

#[tokio::test]
async fn todo_parent_and_children_complete_in_one_node() {
    let dir = tempfile::tempdir().unwrap();
    let client=Arc::new(MockChatClient::new()
        .push_script(done(r#"{"operation":"dispatch","todos":[{"todo_id":"t1","context_mode":"new"}],"reason":"ready"}"#))
        .push_script(done(r#"{"status":"candidate","summary":"done","result":"ok","verification":"checked","evidence_refs":[],"recovery_context":{"summary":"done","refs":[]}}"#))
        .push_script(done(r#"{"operation":"accept","reason":"meets criteria","mark_milestone":true}"#))
        .push_script(done(r#"{"operation":"complete","reason":"all passed"}"#)));
    let node = worker(dir.path(), client).await;
    let spec = json!({"schema_version":1,"id":"wf-test","name":"test","objective":"finish item","constraints":[],"todos":[{"id":"t1","title":"step","requirement_background":"test","instructions":"return candidate","depends_on":[],"agent":"act","max_attempts":2,"acceptance":{"criteria":"candidate exists"}}]});
    let reply = node
        .handle(NodeOperation::Create {
            assignment: assignment(
                &node,
                "todos-local",
                ExecutionKind::Todos,
                json!({}),
                Some(spec),
            ),
        })
        .await;
    assert_eq!(reply.status, 200, "{:?}", reply);
    let detail = settled(&node, "todos-local").await;
    assert_eq!(detail["execution"]["status"], "done", "{detail}");
    assert_eq!(detail["workflow"]["items"][0]["status"], "passed");
    assert!(dir
        .path()
        .join("node/todos/todos-local/execution.json")
        .is_file());
    assert!(node
        .indexes()
        .await
        .unwrap()
        .iter()
        .all(|i| i.node_id == node.registration().id));
    node.shutdown().await.unwrap();
}

#[tokio::test]
async fn maintenance_agent_has_real_local_query_tool() {
    let dir = tempfile::tempdir().unwrap();
    let client = Arc::new(
        MockChatClient::new()
            .push_script(vec![LlmEvent::Completed {
                text: "query status".into(),
                tool_calls: vec![CompletedToolCall {
                    id: "query".into(),
                    name: "node_maintenance".into(),
                    input: json!({"action":"status"}),
                }],
                usage: None,
            }])
            .with_default(done("status queried")),
    );
    let node = worker(dir.path(), client.clone()).await;
    let reply = node
        .handle(NodeOperation::Maintenance {
            command: ExecutionCommand {
                action: "ask".into(),
                input: json!({"id":"maintenance-tool","prompt":"查询节点状态"}),
            },
        })
        .await;
    assert_eq!(reply.status, 200, "{:?}", reply);
    let detail = settled(&node, "maintenance-tool").await;
    assert_eq!(detail["execution"]["status"], "idle", "{detail}");
    assert!(dir
        .path()
        .join("node/maintenance/maintenance-tool/execution.json")
        .is_file());
    assert!(support::messages::message_page_text(&detail)
        .contains(&node.registration().maintenance_agent_id));
    assert!(client
        .requests()
        .iter()
        .any(|r| serde_json::to_string(&r.tools)
            .unwrap()
            .contains("node_maintenance")));
    node.shutdown().await.unwrap();
}

#[tokio::test]
async fn dag_cancel_interrupts_wasm_step_and_releases_node_capacity() {
    let dir = tempfile::tempdir().unwrap();
    let node = worker(dir.path(), mock()).await;
    let id = "dag-cancel-wasm";
    // A spinning wasm step: only epoch interruption (cancel/timeout) ends it.
    support::stage_spin_wasm(&dir.path().join("node"));
    let spec = json!({"name":"cancel-wasm","steps":[{"name":"loop","kind":{"type":"wasm","command":"spin.wasm"}}]});
    assert_eq!(
        node.handle(NodeOperation::Create {
            assignment: assignment(&node, id, ExecutionKind::Dag, json!({}), Some(spec)),
        })
        .await
        .status,
        200
    );
    // context.json lands just before the module starts — the run signal.
    let started = dir.path().join(format!("node/dag/{id}/loop/context.json"));
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while !started.is_file() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        node.handle(NodeOperation::Command {
            execution: ExecutionRef {
                id: id.into(),
                kind: ExecutionKind::Dag,
            },
            command: ExecutionCommand {
                action: "cancel".into(),
                input: json!({}),
            }
        })
        .await
        .status,
        200
    );
    let detail = settled(&node, id).await;
    assert_eq!(detail["execution"]["status"], "cancelled", "{detail}");
    assert_eq!(node.snapshot().active_runs, 0);
    let meta: Value = serde_json::from_slice(
        &std::fs::read(dir.path().join(format!("node/dag/{id}/loop/meta.json"))).unwrap(),
    )
    .unwrap();
    assert_eq!(meta["outcome"], "cancelled");
    node.shutdown().await.unwrap();
}
