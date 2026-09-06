#[path = "../support/mod.rs"]
mod support;

use opencoder_core::fleet::*;
use opencoder_llm::{LlmEvent, MockChatClient};
use opencoder_node::fleet::NodeService;
use opencoder_worker::Worker;
use serde_json::json;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use support::{
    assignment, drain_options as options, drain_runtime as runtime, settled, InterruptClient,
};

fn execution(id: &str) -> ExecutionRef {
    ExecutionRef {
        id: id.into(),
        kind: ExecutionKind::Agent,
    }
}

#[tokio::test]
async fn storage_and_durable_freeze_reject_every_new_work_entry() {
    let dir = tempfile::tempdir().unwrap();
    let low = Arc::new(AtomicBool::new(true));
    let client = Arc::new(
        MockChatClient::new().with_default(vec![LlmEvent::Completed {
            text: "done".into(),
            tool_calls: vec![],
            usage: None,
        }]),
    );
    let worker = Worker::open_with_runtime(
        options(dir.path()),
        Some(client.clone()),
        runtime(low.clone()),
    )
    .await
    .unwrap();
    assert!(!worker.snapshot().ready);
    let request = assignment(
        &worker,
        "agent-health",
        ExecutionKind::Agent,
        json!({"prompt":""}),
        None,
    );
    assert_eq!(
        worker
            .handle(NodeOperation::Create {
                assignment: request.clone(),
            })
            .await
            .status,
        503
    );
    low.store(false, Ordering::SeqCst);
    assert!(worker.snapshot().ready);
    assert_eq!(
        worker
            .handle(NodeOperation::Create {
                assignment: request.clone(),
            })
            .await
            .status,
        200
    );
    let _ = settled(&worker, "agent-health").await;
    assert_eq!(
        worker
            .handle(NodeOperation::Command {
                execution: execution("agent-health"),
                command: ExecutionCommand {
                    action: "interrupt".into(),
                    input: json!({}),
                },
            })
            .await
            .status,
        200
    );
    assert_eq!(
        worker
            .handle(NodeOperation::Command {
                execution: execution("agent-health"),
                command: ExecutionCommand {
                    action: "resume".into(),
                    input: json!({}),
                },
            })
            .await
            .status,
        200,
        "an execution without an initial prompt still launches on resume"
    );
    let _ = settled(&worker, "agent-health").await;
    let frozen = worker
        .handle(NodeOperation::Admission {
            command: NodeAdmissionCommand::Freeze,
        })
        .await;
    assert_eq!(frozen.status, 200, "{frozen:?}");
    assert_eq!(frozen.body["mode"], "frozen");
    assert!(!worker.snapshot().ready);
    assert_eq!(
        worker
            .handle(NodeOperation::Create {
                assignment: request,
            })
            .await
            .status,
        200,
        "an identical acceptance retry is not new work"
    );
    let prompt = worker
        .handle(NodeOperation::Command {
            execution: execution("agent-health"),
            command: ExecutionCommand {
                action: "http".into(),
                input: json!({"method":"POST","tail":"prompt?delivery=queue","body":{"prompt":"later"}}),
            },
        })
        .await;
    assert_eq!(prompt.status, 503, "{prompt:?}");
    for command in [
        ExecutionCommand {
            action: "ask".into(),
            input: json!({"prompt":"new maintenance work"}),
        },
        ExecutionCommand {
            action: "configure".into(),
            input: json!({"model":"must-not-change"}),
        },
    ] {
        let reply = worker.handle(NodeOperation::Maintenance { command }).await;
        assert_eq!(reply.status, 503, "{reply:?}");
    }
    worker.shutdown().await.unwrap();
    drop(worker);

    let reopened = Worker::open_with_runtime(options(dir.path()), Some(client), runtime(low))
        .await
        .unwrap();
    let status = reopened
        .handle(NodeOperation::Admission {
            command: NodeAdmissionCommand::Status,
        })
        .await;
    assert_eq!(
        status.body["mode"], "frozen",
        "frozen mode must survive restart"
    );
    let reopened_reply = reopened
        .handle(NodeOperation::Admission {
            command: NodeAdmissionCommand::Reopen,
        })
        .await;
    assert_eq!(reopened_reply.status, 200, "{reopened_reply:?}");
    assert!(reopened.snapshot().ready);
    reopened.shutdown().await.unwrap();
}

#[tokio::test]
async fn drain_persists_interrupt_and_restart_never_replays() {
    let dir = tempfile::tempdir().unwrap();
    let low = Arc::new(AtomicBool::new(false));
    let release = Arc::new(tokio::sync::Notify::new());
    let client = Arc::new(MockChatClient::new().push_hang(release).with_default(vec![
        LlmEvent::Completed {
            text: "resumed once".into(),
            tool_calls: vec![],
            usage: None,
        },
    ]));
    let first = Worker::open_with_runtime(
        options(dir.path()),
        Some(client.clone()),
        runtime(low.clone()),
    )
    .await
    .unwrap();
    assert_eq!(
        first
            .handle(NodeOperation::Create {
                assignment: assignment(
                    &first,
                    "agent-drain",
                    ExecutionKind::Agent,
                    json!({"prompt":"wait","title":"drain fixture"}),
                    None,
                ),
            })
            .await
            .status,
        200
    );
    first.drain_shutdown().await.unwrap();
    let detail = first
        .handle(NodeOperation::Inspect {
            execution: execution("agent-drain"),
        })
        .await;
    assert_eq!(detail.body["execution"]["status"], "interrupted");
    let record: serde_json::Value = serde_json::from_slice(
        &std::fs::read(dir.path().join("node/agent/agent-drain/execution.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(record["lifecycle"]["stop_intent"], "interrupt");
    let calls = client.call_count();
    drop(first);

    let second = Worker::open_with_runtime(options(dir.path()), Some(client.clone()), runtime(low))
        .await
        .unwrap();
    assert_eq!(client.call_count(), calls, "restart must not replay work");
    assert_eq!(
        second
            .handle(NodeOperation::Command {
                execution: execution("agent-drain"),
                command: ExecutionCommand {
                    action: "resume".into(),
                    input: json!({}),
                },
            })
            .await
            .status,
        503
    );
    second.reopen_admission().await.unwrap();
    assert_eq!(
        second
            .handle(NodeOperation::Command {
                execution: execution("agent-drain"),
                command: ExecutionCommand {
                    action: "resume".into(),
                    input: json!({}),
                },
            })
            .await
            .status,
        200
    );
    assert_eq!(
        settled(&second, "agent-drain").await["execution"]["status"],
        "idle"
    );
    assert_eq!(
        client.call_count(),
        calls + 1,
        "requests: {:#?}",
        client.requests()
    );
    second.shutdown().await.unwrap();
}

#[tokio::test]
async fn low_storage_blocks_new_work_while_existing_work_finishes_naturally() {
    let dir = tempfile::tempdir().unwrap();
    let low = Arc::new(AtomicBool::new(false));
    let finish = Arc::new(tokio::sync::Notify::new());
    let client = Arc::new(InterruptClient {
        calls: std::sync::atomic::AtomicUsize::new(0),
        first_release: finish.clone(),
        resumed_release: Arc::new(tokio::sync::Notify::new()),
    });
    let mut worker_runtime = runtime(low.clone());
    worker_runtime.drain.natural_grace = Duration::from_secs(2);
    let worker =
        Worker::open_with_runtime(options(dir.path()), Some(client.clone()), worker_runtime)
            .await
            .unwrap();
    assert_eq!(
        worker
            .handle(NodeOperation::Create {
                assignment: assignment(
                    &worker,
                    "agent-natural-drain",
                    ExecutionKind::Agent,
                    json!({
                        "prompt":"finish before drain deadline",
                        "title":"natural drain fixture"
                    }),
                    None,
                ),
            })
            .await
            .status,
        200
    );
    tokio::time::timeout(Duration::from_secs(2), async {
        while client.calls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();

    low.store(true, Ordering::SeqCst);
    assert!(!worker.snapshot().ready);
    assert_eq!(
        worker
            .handle(NodeOperation::Create {
                assignment: assignment(
                    &worker,
                    "agent-rejected-low-storage",
                    ExecutionKind::Agent,
                    json!({"prompt":"must not start"}),
                    None,
                ),
            })
            .await
            .status,
        503
    );

    let draining = {
        let worker = worker.clone();
        tokio::spawn(async move { worker.drain_shutdown().await })
    };
    finish.notify_one();
    draining.await.unwrap().unwrap();
    let detail = worker
        .handle(NodeOperation::Inspect {
            execution: execution("agent-natural-drain"),
        })
        .await;
    assert_eq!(
        detail.body["execution"]["status"],
        "interrupted",
        "a naturally completed interactive turn remains unfinished and is durably interrupted: {detail:?}"
    );
    assert_eq!(client.calls.load(Ordering::SeqCst), 1);
    let record: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            dir.path()
                .join("node/agent/agent-natural-drain/execution.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(record["lifecycle"]["stop_intent"], "interrupt");
    worker.shutdown().await.unwrap();
}

#[tokio::test]
async fn drain_cancels_a_hanging_title_request_without_a_seeded_title() {
    let dir = tempfile::tempdir().unwrap();
    let low = Arc::new(AtomicBool::new(false));
    let never_release = Arc::new(tokio::sync::Notify::new());
    let client = Arc::new(
        MockChatClient::new()
            .push_script(vec![LlmEvent::Completed {
                text: "primary answer".into(),
                tool_calls: vec![],
                usage: None,
            }])
            .push_hang(never_release),
    );
    let worker = Worker::open_with_runtime(options(dir.path()), Some(client.clone()), runtime(low))
        .await
        .unwrap();
    let created = worker
        .handle(NodeOperation::Create {
            assignment: assignment(
                &worker,
                "agent-hanging-title",
                ExecutionKind::Agent,
                json!({"prompt":"complete the primary turn"}),
                None,
            ),
        })
        .await;
    assert_eq!(created.status, 200, "{created:?}");
    tokio::time::timeout(Duration::from_secs(2), async {
        while client.call_count() < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("title request must start and remain pending");

    tokio::time::timeout(Duration::from_secs(2), worker.drain_shutdown())
        .await
        .expect("drain must cancel the hanging title request")
        .unwrap();
    assert_eq!(
        worker
            .handle(NodeOperation::Inspect {
                execution: execution("agent-hanging-title"),
            })
            .await
            .body["execution"]["status"],
        "interrupted"
    );
}
