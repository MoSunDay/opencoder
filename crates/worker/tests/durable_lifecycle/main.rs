#[path = "../support/mod.rs"]
mod support;

use opencoder_core::fleet::*;
use opencoder_llm::MockChatClient;
use opencoder_node::fleet::NodeService;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use support::{assignment, settled, worker, worker_with_client, Fleet, InterruptClient};
fn reference(id: &str, kind: ExecutionKind) -> ExecutionRef {
    ExecutionRef {
        id: id.into(),
        kind,
    }
}

async fn command(
    worker: &opencoder_worker::Worker,
    id: &str,
    kind: ExecutionKind,
    action: &str,
) -> RpcReply {
    worker
        .handle(NodeOperation::Command {
            execution: reference(id, kind),
            command: ExecutionCommand {
                action: action.into(),
                input: json!({}),
            },
        })
        .await
}

async fn wait_for_calls(client: &MockChatClient, expected: usize) {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while client.call_count() < expected {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

fn read_record(root: &std::path::Path, kind: &str, id: &str) -> Value {
    serde_json::from_slice(
        &std::fs::read(root.join(format!("node/{kind}/{id}/execution.json"))).unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn idle_cancel_is_durable_terminal_and_repeated_stop_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let client = support::mock();
    let first = worker(dir.path(), client.clone()).await;
    let id = "agent-idle-cancel";
    assert_eq!(
        first
            .handle(NodeOperation::Create {
                assignment: assignment(
                    &first,
                    id,
                    ExecutionKind::Agent,
                    json!({"prompt":"finish a turn"}),
                    None,
                ),
            })
            .await
            .status,
        200
    );
    assert_eq!(settled(&first, id).await["execution"]["status"], "idle");
    let relayed_interrupt = first
        .handle(NodeOperation::Command {
            execution: reference(id, ExecutionKind::Agent),
            command: ExecutionCommand {
                action: "http".into(),
                input: json!({"method":"POST","tail":"interrupt?source=relay","body":{}}),
            },
        })
        .await;
    assert_eq!(relayed_interrupt.body["status"], "interrupted");
    assert_eq!(
        read_record(dir.path(), "agent", id)["lifecycle"]["stop_intent"],
        "interrupt"
    );

    let cancelled = command(&first, id, ExecutionKind::Agent, "cancel").await;
    assert_eq!(cancelled.status, 200, "{cancelled:?}");
    assert_eq!(cancelled.body["status"], "cancelled");
    let before = read_record(dir.path(), "agent", id);
    assert_eq!(before["lifecycle"]["stop_intent"], "cancel");
    assert_eq!(before["assignment"]["index"]["status"], "cancelled");

    for action in ["cancel", "interrupt"] {
        let reply = command(&first, id, ExecutionKind::Agent, action).await;
        assert_eq!(reply.status, 200, "{reply:?}");
        assert_eq!(reply.body["status"], "cancelled");
    }
    let after = read_record(dir.path(), "agent", id);
    assert_eq!(after["events"], before["events"]);
    assert_eq!(
        command(&first, id, ExecutionKind::Agent, "resume")
            .await
            .status,
        409
    );
    assert_eq!(
        support::prompt(&first, id, "must stay cancelled")
            .await
            .status,
        409
    );
    first.shutdown().await.unwrap();
    drop(first);

    let second = worker(dir.path(), client).await;
    let detail = second
        .handle(NodeOperation::Inspect {
            execution: reference(id, ExecutionKind::Agent),
        })
        .await;
    assert_eq!(detail.body["execution"]["status"], "cancelled");
    assert_eq!(
        command(&second, id, ExecutionKind::Agent, "resume")
            .await
            .status,
        409
    );
    second.shutdown().await.unwrap();
}

#[tokio::test]
async fn cancel_intent_survives_process_loss_and_cannot_recover_as_interrupted() {
    let dir = tempfile::tempdir().unwrap();
    let client = support::mock();
    let first = worker(dir.path(), client.clone()).await;
    let id = "agent-cancel-crash";
    let mut accepted = assignment(
        &first,
        id,
        ExecutionKind::Agent,
        json!({"prompt":"must not replay"}),
        None,
    );
    accepted.index.status = ExecutionStatus::Cancelling;
    let record = json!({
        "assignment": accepted,
        "result": null,
        "error": null,
        "events": [],
        "lifecycle": {"stop_intent":"cancel"}
    });
    let path = dir.path().join(format!("node/agent/{id}/execution.json"));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    opencoder_core::atomic_write(&path, &serde_json::to_vec(&record).unwrap()).unwrap();
    drop(first);

    let recovered = worker(dir.path(), client.clone()).await;
    let detail = recovered
        .handle(NodeOperation::Inspect {
            execution: reference(id, ExecutionKind::Agent),
        })
        .await;
    assert_eq!(detail.body["execution"]["status"], "cancelled");
    assert_eq!(client.call_count(), 0, "recovery must never auto replay");
    assert_eq!(
        command(&recovered, id, ExecutionKind::Agent, "resume")
            .await
            .status,
        409
    );
    recovered.shutdown().await.unwrap();
}

#[tokio::test]
async fn interrupt_is_resumable_once_and_late_interrupt_cannot_downgrade_cancel() {
    let dir = tempfile::tempdir().unwrap();
    let first_hang = Arc::new(tokio::sync::Notify::new());
    let resumed_hang = Arc::new(tokio::sync::Notify::new());
    let client = Arc::new(InterruptClient {
        calls: AtomicUsize::new(0),
        first_release: first_hang.clone(),
        resumed_release: resumed_hang.clone(),
    });
    let first = worker_with_client(dir.path(), client.clone()).await;
    let id = "agent-interrupt-resume";
    assert_eq!(
        first
            .handle(NodeOperation::Create {
                assignment: assignment(
                    &first,
                    id,
                    ExecutionKind::Agent,
                    json!({"prompt":"resume once"}),
                    None,
                ),
            })
            .await
            .status,
        200
    );
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while client.calls.load(Ordering::SeqCst) < 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let interrupted = command(&first, id, ExecutionKind::Agent, "interrupt").await;
    assert_eq!(interrupted.status, 200, "{interrupted:?}");
    assert_eq!(
        settled(&first, id).await["execution"]["status"],
        "interrupted"
    );
    assert_eq!(
        read_record(dir.path(), "agent", id)["lifecycle"]["stop_intent"],
        "interrupt"
    );
    first_hang.notify_waiters();
    let resumed = command(&first, id, ExecutionKind::Agent, "resume").await;
    assert_eq!(resumed.status, 200, "{resumed:?}");
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while client.calls.load(Ordering::SeqCst) < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        command(&first, id, ExecutionKind::Agent, "resume")
            .await
            .status,
        409,
        "a concurrent second resume must not start another driver"
    );
    resumed_hang.notify_waiters();
    assert_eq!(settled(&first, id).await["execution"]["status"], "idle");
    let record = read_record(dir.path(), "agent", id);
    let starts = record["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event["data"]["status"] == "running")
        .count();
    assert_eq!(starts, 2, "initial run plus one explicit resume: {record}");

    let cancel = command(&first, id, ExecutionKind::Agent, "cancel").await;
    assert_eq!(cancel.body["status"], "cancelled");
    let late = command(&first, id, ExecutionKind::Agent, "interrupt").await;
    assert_eq!(late.body["status"], "cancelled");
    first.shutdown().await.unwrap();
}

#[tokio::test]
async fn cancelled_project_execution_cannot_resume() {
    let hang = Arc::new(tokio::sync::Notify::new());
    let client = Arc::new(MockChatClient::new().push_hang(hang.clone()));
    let fleet = Fleet::new(1, client.clone()).await;
    let node = &fleet.nodes[0];
    let id = "project-todo-cancel";
    let todo_id = "todo-cancel";
    let mut project = assignment(
        node,
        id,
        ExecutionKind::Project,
        json!({"action":"plan"}),
        Some(support::project_snapshot(todo_id)),
    );
    project.request.target = Some(todo_id.into());
    assert_eq!(
        node.handle(NodeOperation::Create {
            assignment: project,
        })
        .await
        .status,
        200
    );
    wait_for_calls(&client, 1).await;
    let run_id = node
        .indexes()
        .await
        .unwrap()
        .into_iter()
        .find(|index| index.kind == ExecutionKind::Project && index.id != id)
        .unwrap()
        .id;
    let old_run = "prun-old-done";
    support::seed_project_run(
        &fleet.root().join("n0/node/runtime.db"),
        old_run,
        todo_id,
        opencoder_store::ProjectTodoRunStatus::Done,
    )
    .await;
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while fleet.state.fleet.index(&run_id).await.unwrap().is_none()
            || fleet.state.fleet.index(old_run).await.unwrap().is_none()
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let old = fleet
        .call(
            "POST",
            &format!("/api/project/runs/{old_run}/cancel"),
            Value::Null,
        )
        .await;
    assert_eq!(old.body["status"], "done", "{old:?}");
    assert_eq!(
        node.handle(NodeOperation::Inspect {
            execution: reference(id, ExecutionKind::Project),
        })
        .await
        .body["execution"]["status"],
        "running"
    );
    assert_eq!(
        fleet
            .call(
                "POST",
                &format!("/api/project/runs/{run_id}/cancel"),
                Value::Null
            )
            .await
            .status,
        200
    );
    assert_eq!(settled(node, id).await["execution"]["status"], "cancelled");
    assert_eq!(
        command(node, id, ExecutionKind::Project, "resume")
            .await
            .status,
        409
    );
    hang.notify_waiters();
    fleet.shutdown().await;
}

#[tokio::test]
async fn completed_execution_wins_a_late_cancel_without_rewriting_results() {
    let dir = tempfile::tempdir().unwrap();
    let node = worker(dir.path(), support::mock()).await;
    let id = "dag-finish-wins";
    support::stage_stdout_wasm(&dir.path().join("node"), "tool.wasm", "done");
    let spec = json!({
        "name": "finish-wins",
        "steps": [{"name":"done","kind":{"type":"wasm","command":"tool.wasm"}}]
    });
    assert_eq!(
        node.handle(NodeOperation::Create {
            assignment: assignment(&node, id, ExecutionKind::Dag, json!({}), Some(spec)),
        })
        .await
        .status,
        200
    );
    assert_eq!(settled(&node, id).await["execution"]["status"], "done");
    let before = read_record(dir.path(), "dag", id);

    let late = command(&node, id, ExecutionKind::Dag, "cancel").await;
    assert_eq!(late.status, 200, "{late:?}");
    assert_eq!(late.body["status"], "done");
    let after = read_record(dir.path(), "dag", id);
    assert_eq!(after["events"], before["events"]);
    assert_eq!(after["result"], before["result"]);
    node.shutdown().await.unwrap();
}
