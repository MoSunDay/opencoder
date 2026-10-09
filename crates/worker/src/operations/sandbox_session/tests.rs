//! Unit tests for the `run_mode: agent` command interception: sandbox
//! sessions must never reach the host web app with a prompt (a native POST
//! would start a HOST turn), session-shaping POSTs are rejected, and the
//! admitted native input fails closed through the runc preflights — at prompt
//! admission (inside `create::prepare`) and again inside the round. The
//! full container round is covered by the root-package e2e suite.

use crate::{journal::Record, lifecycle::Lifecycle, operations::queue, Worker, WorkerOptions};
use opencoder_core::fleet::*;
use serde_json::{json, Value};

const ID: &str = "sandbox-command-1";

/// A node with an idle `kind=agent` execution for `agent` whose pinned
/// resources root (the execution's agents pool) holds a RESOLVABLE card —
/// `current.prompt` pointing at a prompts pool version with a soul, so
/// admission's `resolve_agent` succeeds; `run_mode` picks host vs sandbox.
async fn worker_with_execution(agent: &str, run_mode: &str) -> (Worker, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let _config = opencoder_core::config::scoped_config_home(dir.path().join("config-home"));
    let worker = Worker::open(
        WorkerOptions {
            name: "sandbox-test".into(),
            workdir: dir.path().join("work"),
            data_dir: dir.path().join("node"),
            workflow_root: None,
            max_runs: Some(1),
            dag: true,
        },
        Some(std::sync::Arc::new(opencoder_llm::MockChatClient::new())),
    )
    .await
    .unwrap();
    let pool = worker
        .inner
        .layout
        .resources_dir(ExecutionKind::Agent, ID)
        .unwrap();
    let card = pool.join(agent);
    std::fs::create_dir_all(&card).unwrap();
    std::fs::write(
        card.join("meta.json"),
        json!({"run_mode": run_mode, "current": {"prompt": "p1"}}).to_string(),
    )
    .unwrap();
    let prompt = pool.join("prompts").join("p1");
    std::fs::create_dir_all(prompt.join("v1")).unwrap();
    std::fs::write(prompt.join("meta.json"), r#"{"current":1}"#).unwrap();
    std::fs::write(prompt.join("v1").join("soul.md"), "# soul\n").unwrap();
    let record = Record {
        annotations: Value::Null,
        queue: None,
        assignment: Assignment {
            private_context: None,
            runtime: None,
            codex: None,
            index: ExecutionIndex {
                id: ID.into(),
                kind: ExecutionKind::Agent,
                node_id: worker.inner.registration.id.clone(),
                created_at: 1,
                status: ExecutionStatus::Idle,
            },
            request: CreateExecution {
                id: ID.into(),
                kind: ExecutionKind::Agent,
                target: Some(agent.into()),
                node_id: None,
                input: json!({}),
            },
            definition: None,
        },
        result: json!({}),
        error: None,
        events: vec![],
        lifecycle: Lifecycle::default(),
    };
    worker.inner.journal.lock().await.save(record).unwrap();
    (worker, dir)
}

async fn command(worker: &Worker, action: &str, input: Value) -> anyhow::Result<RpcReply> {
    crate::operations::command::command(
        worker,
        &ExecutionRef {
            id: ID.into(),
            kind: ExecutionKind::Agent,
        },
        ExecutionCommand {
            action: action.into(),
            input,
        },
    )
    .await
}

/// Poll until the execution leaves Running; return its terminal (status,
/// error).
async fn settle(worker: &Worker) -> (ExecutionStatus, String) {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let record = worker.inner.journal.lock().await.records[ID].clone();
            if record.assignment.index.status != ExecutionStatus::Running
                && record.assignment.index.status != ExecutionStatus::Idle
                && record.assignment.index.status != ExecutionStatus::Pending
            {
                return (
                    record.assignment.index.status,
                    record.error.unwrap_or_default(),
                );
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("execution settles")
}

#[tokio::test]
async fn sandbox_session_rejects_host_only_session_operations() {
    let (worker, _dir) = worker_with_execution("myagent", "agent").await;
    for tail in ["steer", "queue", "compact", "handoff"] {
        let reply = command(&worker, "http", json!({"method": "POST", "tail": tail}))
            .await
            .unwrap();
        assert_eq!(
            reply.status, 409,
            "{tail}: {reply:?} (must not fake host-session success)"
        );
        assert!(reply.body.to_string().contains("sandbox conversations"));
    }
    // GETs keep flowing through the native app (store-backed reads).
    let reply = command(&worker, "http", json!({"method": "GET", "tail": ""}))
        .await
        .unwrap();
    assert_eq!(reply.status, 200, "{reply:?}");
}

#[tokio::test]
async fn sandbox_prompt_fails_closed_without_a_runtime() {
    let (worker, _dir) = worker_with_execution("myagent", "agent").await;
    // A blank prompt is rejected before any state change.
    let reply = command(&worker, "prompt", json!({"prompt": "   "}))
        .await
        .unwrap();
    assert_eq!(reply.status, 400, "{reply:?}");
    // A real prompt is persisted in the native input store, then runc
    // preflight rejects it — this node has no provisioned sandbox runtime,
    // and a sandbox request must never fall back to a host turn.
    let error = command(
        &worker,
        "prompt",
        json!({"prompt": "turn 2", "input_id": "followup"}),
    )
    .await
    .expect_err("sandbox prompt must fail closed without a runtime");
    let error = format!("{error:#}");
    assert!(
        error.contains("runc executable unavailable") || error.contains("runc rootfs unavailable"),
        "unexpected rejection: {error}"
    );
    let record = worker.inner.journal.lock().await.records[ID].clone();
    assert!(record.assignment.request.input["prompt"].is_null());
    let runtime = crate::workloads::agent_runc::inputs::open(&worker, &record, false)
        .await
        .unwrap();
    use opencoder_store::Store;
    assert_eq!(
        runtime
            .pending_inputs(ID, opencoder_store::Delivery::Steer)
            .await
            .unwrap()[0]
            .prompt,
        "turn 2"
    );
    // The host may expose metadata, but never runs the turn.
    assert!(worker
        .inner
        .state
        .store
        .events_after(ID, 0)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn queued_sandbox_prompt_replays_as_a_round_not_a_host_turn() {
    let (worker, _dir) = worker_with_execution("myagent", "agent").await;
    let pool = worker
        .inner
        .layout
        .resources_dir(ExecutionKind::Agent, ID)
        .unwrap();
    // A queued follow-up exactly as enqueue_with_command would persist it;
    // the replay must drain the native input store, never POST
    // to the host web app.
    let mut config = worker.configuration().unwrap();
    config.agent.agents_dir = Some(pool);
    {
        let mut journal = worker.inner.journal.lock().await;
        let mut record = journal.records[ID].clone();
        record.assignment.index.status = ExecutionStatus::Pending;
        record.queue = Some(Box::new(queue::QueuedRun {
            ticket: None,
            sequence: 1,
            resume: true,
            config,
            command: Some(queue::QueuedCommand {
                tail: "prompt".into(),
                body: json!({"prompt": "queued turn", "input_id": "followup"}),
            }),
        }));
        journal.save(record).unwrap();
    }
    let record = worker.inner.journal.lock().await.records[ID].clone();
    let runtime = crate::workloads::agent_runc::inputs::open(&worker, &record, false)
        .await
        .unwrap();
    crate::workloads::agent_runc::inputs::admit(
        runtime.as_ref(),
        ID,
        &json!({"prompt":"queued turn","input_id":"followup"}),
    )
    .await
    .unwrap();
    queue::dispatch_locked(&worker).await.unwrap();
    let (status, error) = settle(&worker).await;
    assert_eq!(status, ExecutionStatus::Error);
    assert!(
        error.contains("runc executable unavailable") || error.contains("runc rootfs unavailable"),
        "unexpected workload error: {error}"
    );
    // The native input survives the failed preflight for a later retry.
    use opencoder_store::Store;
    assert_eq!(
        runtime
            .pending_inputs(ID, opencoder_store::Delivery::Steer)
            .await
            .unwrap()[0]
            .prompt,
        "queued turn"
    );
    assert!(worker.inner.journal.lock().await.records[ID]
        .assignment
        .request
        .input["prompt"]
        .is_null());
    assert!(worker
        .inner
        .state
        .store
        .events_after(ID, 0)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn host_sessions_keep_the_native_prompt_path() {
    // A resolving `run_mode: operator` card keeps the native host path:
    // the follow-up flows through the web app (store-owned turn
    // admission) and the request input is never rewritten.
    let (worker, _dir) = worker_with_execution("hostagent", "operator").await;
    let reply = command(
        &worker,
        "prompt",
        json!({"input_id": "followup-1", "prompt": "second turn"}),
    )
    .await
    .unwrap();
    assert_eq!(reply.status, 200, "{reply:?}");
    assert!(
        worker.inner.journal.lock().await.records[ID]
            .assignment
            .request
            .input["prompt"]
            .is_null(),
        "native follow-ups must not rewrite the request input"
    );
}

#[tokio::test]
async fn running_sandbox_admits_idempotent_steer_and_queue_in_native_store() {
    use crate::workloads::agent_runc::inputs;
    use opencoder_store::{Delivery, Store};
    let (worker, _dir) = worker_with_execution("myagent", "agent").await;
    worker
        .inner
        .active
        .lock()
        .await
        .insert(ID.into(), tokio_util::sync::CancellationToken::new());
    let first = command(
        &worker,
        "steer",
        json!({"prompt":"more context","input_id":"stable"}),
    )
    .await
    .unwrap();
    assert_eq!(first.status, 200, "{first:?}");
    let duplicate = command(
        &worker,
        "steer",
        json!({"prompt":"more context","input_id":"stable"}),
    )
    .await
    .unwrap();
    assert_eq!(duplicate.body["inserted"], false);
    assert_eq!(duplicate.body["seq"], first.body["seq"]);
    assert_eq!(
        command(
            &worker,
            "steer",
            json!({"prompt":"different","input_id":"stable"})
        )
        .await
        .unwrap()
        .status,
        409
    );
    assert_eq!(
        command(
            &worker,
            "queue",
            json!({"prompt":"next task","input_id":"next"})
        )
        .await
        .unwrap()
        .status,
        200
    );
    let record = worker.inner.journal.lock().await.records[ID].clone();
    let runtime = inputs::open(&worker, &record, false).await.unwrap();
    assert_eq!(
        runtime
            .pending_inputs(ID, Delivery::Steer)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        runtime
            .pending_inputs(ID, Delivery::Queue)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        worker
            .inner
            .state
            .store
            .pending_inputs(ID, Delivery::Steer)
            .await
            .unwrap()
            .is_empty(),
        "host must not consume the sandbox input"
    );
    let list = command(
        &worker,
        "http",
        json!({"method":"GET","tail":"inputs?delivery=queue"}),
    )
    .await
    .unwrap();
    assert_eq!(list.body["inputs"][0]["prompt"], "next task");
    // Reopening a runtime connection retains admission and retry receipts.
    let reopened = opencoder_store::LibsqlStore::open(
        inputs::directory(&worker, ID, false)
            .unwrap()
            .join("runtime.db"),
    )
    .await
    .unwrap();
    assert_eq!(
        reopened
            .pending_inputs(ID, Delivery::Queue)
            .await
            .unwrap()
            .len(),
        1
    );
    let receipt = inputs::admit(
        &reopened,
        ID,
        &json!({"prompt":"next task","input_id":"next","delivery":"queue"}),
    )
    .await
    .unwrap();
    assert_eq!(receipt.body["inserted"], false);
}
