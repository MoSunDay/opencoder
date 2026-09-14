#[path = "../support/mod.rs"]
mod support;
use opencoder_core::fleet::*;
use opencoder_core::Role;
use opencoder_llm::{LlmEvent, MockChatClient};
use opencoder_node::fleet::NodeService;
use opencoder_store::{LibsqlStore, Store};
use opencoder_worker::Worker;
use serde_json::{json, Value};
use std::sync::Arc;
use support::*;

#[tokio::test]
async fn durable_acceptance_deduplicates_and_details_stay_on_node() {
    let dir = tempfile::tempdir().unwrap();
    let client = mock();
    let worker = worker(dir.path(), client.clone()).await;
    let assignment = assignment(
        &worker,
        "agent-durable",
        ExecutionKind::Agent,
        json!({"prompt":"private workload"}),
        None,
    );
    let accepted = worker
        .handle(NodeOperation::Create {
            assignment: assignment.clone(),
        })
        .await;
    assert_eq!(accepted.status, 200, "{:?}", accepted);
    let journal: Value = serde_json::from_slice(
        &std::fs::read(dir.path().join("node/agent/agent-durable/execution.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        journal["assignment"]["request"]["input"]["prompt"],
        "private workload"
    );
    let detail = settled(&worker, "agent-durable").await;
    assert_eq!(detail["execution"]["status"], "idle", "{detail}");
    assert!(!detail["session"]["messages"]["chunks"]
        .as_array()
        .unwrap()
        .is_empty());
    let persisted = LibsqlStore::open(dir.path().join("node/runtime.db"))
        .await
        .unwrap()
        .load_messages("agent-durable")
        .await
        .unwrap();
    assert!(persisted.iter().any(|message| {
        message.role == Role::Assistant && message.text().contains("node-owned answer")
    }));
    let calls = client.call_count();
    assert_eq!(
        worker
            .handle(NodeOperation::Create {
                assignment: assignment.clone()
            })
            .await
            .status,
        200
    );
    assert_eq!(client.call_count(), calls);
    let wrong_kind = worker
        .handle(NodeOperation::Inspect {
            execution: ExecutionRef {
                id: "agent-durable".into(),
                kind: ExecutionKind::Team,
            },
        })
        .await;
    assert_eq!(wrong_kind.status, 409, "{wrong_kind:?}");
    let mut mismatched_assignment = assignment.clone();
    mismatched_assignment.index.kind = ExecutionKind::Team;
    assert_eq!(
        worker
            .handle(NodeOperation::Create {
                assignment: mismatched_assignment
            })
            .await
            .status,
        409
    );
    let mut conflict = assignment;
    conflict.request.input = json!({"prompt":"different"});
    assert_eq!(
        worker
            .handle(NodeOperation::Create {
                assignment: conflict
            })
            .await
            .status,
        409
    );
    let events = worker
        .handle(NodeOperation::Events {
            execution: ExecutionRef {
                id: "agent-durable".into(),
                kind: ExecutionKind::Agent,
            },
            after: 0,
        })
        .await;
    assert!(events.body["events"].as_array().unwrap().len() > 1);
    assert_eq!(
        prompt(&worker, "agent-durable", "followup").await.status,
        200
    );
    let _ = settled(&worker, "agent-durable").await;
    assert!(client.call_count() > calls);
    worker.shutdown().await.unwrap();
}

#[tokio::test]
async fn restart_marks_unfinished_work_interrupted_and_requires_explicit_resume() {
    let dir = tempfile::tempdir().unwrap();
    let client = mock();
    let first = worker(dir.path(), client.clone()).await;
    let assignment = assignment(
        &first,
        "agent-restart",
        ExecutionKind::Agent,
        json!({"prompt":"recover once"}),
        None,
    );
    let node = first.registration().id;
    // Simulate the exact durable-accept / process-loss boundary before launch.
    let mut legacy = json!({"assignment":assignment,"result":null,"error":null,"events":[]});
    legacy["assignment"]["index"]
        .as_object_mut()
        .unwrap()
        .remove("kind");
    std::fs::create_dir_all(dir.path().join("node/executions")).unwrap();
    std::fs::write(
        dir.path().join("node/executions/agent-restart.json"),
        serde_json::to_vec(&legacy).unwrap(),
    )
    .unwrap();
    drop(first);
    let second = worker(dir.path(), client.clone()).await;
    assert_eq!(second.registration().id, node);
    let reply = second
        .handle(NodeOperation::Inspect {
            execution: ExecutionRef {
                id: "agent-restart".into(),
                kind: ExecutionKind::Agent,
            },
        })
        .await;
    assert_eq!(reply.body["execution"]["status"], "interrupted");
    assert_eq!(client.call_count(), 0);
    let reply = second
        .handle(NodeOperation::Command {
            execution: ExecutionRef {
                id: "agent-restart".into(),
                kind: ExecutionKind::Agent,
            },
            command: ExecutionCommand {
                action: "resume".into(),
                input: json!({}),
            },
        })
        .await;
    assert_eq!(reply.status, 200, "{:?}", reply);
    let detail = settled(&second, "agent-restart").await;
    assert_eq!(detail["execution"]["status"], "idle", "{detail}");
    assert!(
        client.call_count() > 0,
        "accepted prompt must be executed after explicit resume"
    );
    second.shutdown().await.unwrap();
}

#[tokio::test]
async fn failed_agent_can_be_explicitly_resumed_without_duplicating_initial_input() {
    let dir = tempfile::tempdir().unwrap();
    let client = Arc::new(
        MockChatClient::new()
            .push_script(vec![LlmEvent::Error("fixture failure".into())])
            .with_default(vec![LlmEvent::Completed {
                text: "recovered".into(),
                tool_calls: vec![],
                usage: None,
            }]),
    );
    let node = worker(dir.path(), client.clone()).await;
    let id = "agent-error-resume";
    assert_eq!(
        node.handle(NodeOperation::Create {
            assignment: assignment(
                &node,
                id,
                ExecutionKind::Agent,
                json!({"prompt":"retry this once","title":"error retry fixture"}),
                None,
            ),
        })
        .await
        .status,
        200
    );
    assert_eq!(settled(&node, id).await["execution"]["status"], "error");
    assert_eq!(client.call_count(), 1);

    let resumed = node
        .handle(NodeOperation::Command {
            execution: ExecutionRef {
                id: id.into(),
                kind: ExecutionKind::Agent,
            },
            command: ExecutionCommand {
                action: "resume".into(),
                input: json!({}),
            },
        })
        .await;
    assert_eq!(resumed.status, 200, "{resumed:?}");
    let detail = settled(&node, id).await;
    assert_eq!(detail["execution"]["status"], "idle", "{detail}");
    assert_eq!(
        client.call_count(),
        2,
        "one explicit retry must call the model exactly once"
    );
    let messages = LibsqlStore::open(dir.path().join("node/runtime.db"))
        .await
        .unwrap()
        .load_messages(id)
        .await
        .unwrap();
    let user_messages: Vec<_> = messages
        .iter()
        .filter(|message| message.role == Role::User)
        .collect();
    assert_eq!(user_messages.len(), 1, "{messages:?}");
    assert_eq!(user_messages[0].display.as_deref(), Some("retry this once"));
    assert!(messages.iter().any(|message| {
        message.role == Role::Assistant && message.text().contains("recovered")
    }));
    let record: Value = serde_json::from_slice(
        &std::fs::read(dir.path().join(format!("node/agent/{id}/execution.json"))).unwrap(),
    )
    .unwrap();
    assert_eq!(
        record["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|event| event["data"]["status"] == "running")
            .count(),
        2,
        "initial driver plus one explicit retry: {record}"
    );
    node.shutdown().await.unwrap();
}

#[tokio::test]
async fn todo_interrupt_compat_route_remains_resumable_once() {
    let first_hang = Arc::new(tokio::sync::Notify::new());
    let resumed_hang = Arc::new(tokio::sync::Notify::new());
    let client = Arc::new(
        MockChatClient::new()
            .push_hang(first_hang.clone())
            .push_hang(resumed_hang.clone()),
    );
    let fleet = Fleet::new(1, client.clone()).await;
    let node = &fleet.nodes[0];
    let id = "todos-interrupt-route";
    let spec = json!({
        "schema_version": 1,
        "id": "wf-interrupt",
        "name": "interrupt",
        "objective": "wait for explicit resume",
        "constraints": [],
        "todos": [{
            "id": "t1",
            "title": "wait",
            "requirement_background": "test",
            "instructions": "finish",
            "depends_on": [],
            "agent": "act",
            "max_attempts": 1,
            "acceptance": {"criteria":"done"}
        }]
    });
    assert_eq!(
        node.handle(NodeOperation::Create {
            assignment: assignment(node, id, ExecutionKind::Todos, json!({}), Some(spec)),
        })
        .await
        .status,
        200
    );
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while fleet.state.fleet.index(id).await.unwrap().is_none() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while client.call_count() < 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();

    let interrupted = fleet
        .call(
            "POST",
            &format!("/api/todo/workflows/{id}/interrupt"),
            Value::Null,
        )
        .await;
    assert_eq!(interrupted.status, 200, "{interrupted:?}");
    assert_eq!(
        settled(node, id).await["execution"]["status"],
        "interrupted"
    );

    let resumed = fleet
        .call(
            "POST",
            &format!("/api/todo/workflows/{id}/resume"),
            Value::Null,
        )
        .await;
    assert_eq!(resumed.status, 200, "{resumed:?}");
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while client.call_count() < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        fleet
            .call(
                "POST",
                &format!("/api/todo/workflows/{id}/resume"),
                Value::Null,
            )
            .await
            .status,
        409
    );
    resumed_hang.notify_waiters();
    first_hang.notify_waiters();
    let _ = settled(node, id).await;
    fleet.shutdown().await;
}

#[tokio::test]
async fn shutdown_waits_for_task_capture_before_immediate_reopen() {
    let dir = tempfile::tempdir().unwrap();
    for attempt in 0..12 {
        let node = worker(dir.path(), mock()).await;
        let id = format!("agent-reopen-{attempt}");
        assert_eq!(
            node.handle(NodeOperation::Create {
                assignment: assignment(
                    &node,
                    &id,
                    ExecutionKind::Agent,
                    json!({"prompt":""}),
                    None,
                ),
            })
            .await
            .status,
            200
        );
        let _ = settled(&node, &id).await;
        node.shutdown().await.unwrap();
        drop(node);
    }
}

/// Worker options with an explicit max-runs budget (the shared `worker()`
/// helper fixes 4): max_runs = 1 pins the single slot so later creations
/// stay durably queued instead of dispatching immediately.
async fn one_slot_worker(
    root: &std::path::Path,
    client: Arc<dyn opencoder_llm::ChatStream>,
) -> Worker {
    let workdir = root.join("work");
    std::fs::create_dir_all(workdir.join(".opencoder")).unwrap();
    std::fs::write(workdir.join(".opencoder/ap.json"), r#"{"mode":"off"}"#).unwrap();
    opencoder_worker::Worker::open(
        opencoder_worker::WorkerOptions {
            name: "test-node".into(),
            workdir,
            data_dir: root.join("node"),
            workflow_root: None,
            max_runs: Some(1),
            dag: true,
        },
        Some(client),
    )
    .await
    .unwrap()
}

async fn status_of(worker: &Worker, id: &str) -> Value {
    worker
        .handle(NodeOperation::Inspect {
            execution: ExecutionRef {
                id: id.into(),
                kind: ExecutionKind::Operator,
            },
        })
        .await
        .body["execution"]["status"]
        .clone()
}

/// Operator records live under `node/operator/` — they must reload with the
/// journal on restart: a finished one keeps its history, an interrupted one
/// stays inspectable, and queued work is picked back up by the scheduler
/// (regression: `ALL_KINDS` once omitted `Operator`, so every operator
/// record silently vanished across a restart).
#[tokio::test]
async fn operator_executions_survive_node_restart() {
    let dir = tempfile::tempdir().unwrap();
    // Chat call #0 blocks until released, keeping its execution Running and
    // the single slot occupied; later calls complete immediately.
    let blocker = Arc::new(InterruptClient {
        calls: std::sync::atomic::AtomicUsize::new(0),
        first_release: Arc::new(tokio::sync::Notify::new()),
        resumed_release: Arc::new(tokio::sync::Notify::new()),
    });
    let first = one_slot_worker(dir.path(), blocker.clone()).await;

    // Occupies the only run slot and stays running (blocked LLM call).
    assert_eq!(
        first
            .handle(NodeOperation::Create {
                assignment: assignment(
                    &first,
                    "operator-restart-running",
                    ExecutionKind::Operator,
                    json!({"prompt":"hang on the host"}),
                    None,
                ),
            })
            .await
            .status,
        200
    );
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if status_of(&first, "operator-restart-running").await == json!("running") {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();

    // These two stay queued (pending, never dispatched) behind the slot.
    for (id, prompt) in [
        ("operator-restart-done", "host once"),
        ("operator-restart-queued", "host twice"),
    ] {
        assert_eq!(
            first
                .handle(NodeOperation::Create {
                    assignment: assignment(
                        &first,
                        id,
                        ExecutionKind::Operator,
                        json!({"prompt": prompt}),
                        None,
                    ),
                })
                .await
                .status,
            200
        );
    }
    assert_eq!(
        status_of(&first, "operator-restart-done").await,
        json!("pending")
    );
    assert_eq!(
        status_of(&first, "operator-restart-queued").await,
        json!("pending")
    );
    // Operator records are journaled in the operator kind root.
    for id in [
        "operator-restart-running",
        "operator-restart-done",
        "operator-restart-queued",
    ] {
        assert!(
            dir.path()
                .join(format!("node/operator/{id}/execution.json"))
                .exists(),
            "operator journal record missing for {id}"
        );
    }

    first.shutdown().await.unwrap();
    drop(first);

    // Restart with an always-completing client: queued operator work must be
    // re-dispatched and finished records must still resolve.
    let second = worker(dir.path(), mock()).await;
    assert_eq!(
        settled(&second, "operator-restart-done").await["execution"]["status"],
        json!("idle")
    );
    assert_eq!(
        settled(&second, "operator-restart-queued").await["execution"]["status"],
        json!("idle")
    );
    // The cancelled execution stays on record (shutdown cancelled it in
    // flight) — the whole point is that it is still addressable, not
    // vanished, after the restart.
    let interrupted = second
        .handle(NodeOperation::Inspect {
            execution: ExecutionRef {
                id: "operator-restart-running".into(),
                kind: ExecutionKind::Operator,
            },
        })
        .await;
    assert_eq!(interrupted.status, 200, "{:?}", interrupted);
    assert_eq!(interrupted.body["execution"]["status"], json!("cancelled"));
    second.shutdown().await.unwrap();
}
