use crate::{operations::create::create, Worker, WorkerOptions};
use opencoder_core::fleet::*;
use serde_json::{json, Value};
use std::{path::Path, sync::Arc, time::Duration};

#[path = "tests/pinned_retry.rs"]
mod pinned_retry;

async fn open(root: &Path) -> Worker {
    Worker::open(
        WorkerOptions {
            name: "preparation-test".into(),
            workdir: root.join("work"),
            data_dir: root.join("node"),
            workflow_root: None,
            max_runs: Some(4),
            dag: true,
        },
        Some(Arc::new(opencoder_llm::MockChatClient::new().with_default(
            vec![opencoder_llm::LlmEvent::Completed {
                text: "accepted".into(),
                tool_calls: vec![],
                usage: None,
            }],
        ))),
    )
    .await
    .unwrap()
}

fn assignment(worker: &Worker, id: &str, kind: ExecutionKind) -> Assignment {
    Assignment {
        runtime: None,
        codex: None,
        definition: None,
        index: ExecutionIndex {
            id: id.into(),
            created_at: 1,
            kind,
            node_id: worker.inner.registration.id.clone(),
            status: ExecutionStatus::Pending,
        },
        request: CreateExecution {
            id: id.into(),
            kind,
            target: (kind == ExecutionKind::Agent).then(|| "act".into()),
            input: json!({}),
            node_id: None,
        },
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn synchronous_preparation_releases_the_only_async_worker() {
    let (started, waiting) = tokio::sync::oneshot::channel();
    let (release, blocking) = std::sync::mpsc::channel();
    let task = tokio::spawn(async move {
        super::preparation::blocking(|| {
            started.send(()).unwrap();
            blocking.recv_timeout(Duration::from_secs(2))
        })
    });
    let responsive = tokio::spawn(async move {
        waiting.await.unwrap();
        tokio::task::yield_now().await;
        release.send(()).unwrap();
    });
    tokio::time::timeout(Duration::from_secs(1), responsive)
        .await
        .expect("cold preparation starved the async worker")
        .unwrap();
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn new_wasi_admission_and_freeze_bypass_cold_resource_waiters() {
    // All resource permits stay held until both operations finish, so the
    // ordering proves bypass. The deadline only detects a deadlock; durable
    // filesystem work on a busy runner need not satisfy a one-second SLO.
    let deadline = Duration::from_secs(10);
    let root = tempfile::tempdir().unwrap();
    let _home = opencoder_core::config::scoped_config_home(root.path().join("home"));
    let worker = open(root.path()).await;
    let cold = assignment(&worker, "agent-cold", ExecutionKind::Agent);
    let lifecycle = worker.preparation_gate(&cold.index.id).await;
    let busy = worker
        .inner
        .resource_preparations
        .acquire_many(4)
        .await
        .unwrap();
    let waiting = worker.clone();
    let task = tokio::spawn(async move { create(&waiting, cold).await });
    tokio::time::timeout(deadline, async {
        while lifecycle.try_lock().is_ok() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();

    let modules = root.path().join("node/dag/_modules");
    std::fs::create_dir_all(&modules).unwrap();
    std::fs::write(
        modules.join("quick.wasm"),
        b"(module (func (export \"_start\")))",
    )
    .unwrap();
    let mut wasm = assignment(&worker, "dag-quick", ExecutionKind::Dag);
    wasm.definition = Some(json!({"name":"quick","steps":[{"name":"execute",
        "kind":{"type":"wasm","command":"quick.wasm"}}]}));
    let reply = tokio::time::timeout(deadline, create(&worker, wasm))
        .await
        .expect("WASI admission waited for unrelated resource capacity")
        .unwrap();
    assert_eq!(reply.status, 200, "{reply:?}");
    assert!(root
        .path()
        .join("node/dag/dag-quick/execution.json")
        .is_file());
    tokio::time::timeout(deadline, worker.freeze_admission())
        .await
        .unwrap()
        .unwrap();
    drop(busy);
    assert_eq!(task.await.unwrap().unwrap().status, 503);
    assert!(!root
        .path()
        .join("node/agent/agent-cold/execution.json")
        .exists());
}

#[tokio::test]
async fn interrupted_preparation_recovers_its_original_request_after_restart() {
    let root = tempfile::tempdir().unwrap();
    let _home = opencoder_core::config::scoped_config_home(root.path().join("home"));
    let worker = open(root.path()).await;
    let original = assignment(&worker, "agent-prepared", ExecutionKind::Agent);
    super::preparation::begin(&worker, original.clone())
        .unwrap()
        .unwrap();
    let marker = root
        .path()
        .join("node/agent/agent-prepared/pending-create.json");
    let frozen = std::fs::read(&marker).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&marker).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    worker.inner.stopping.cancel();
    drop(worker);
    let restarted = open(root.path()).await;
    let mut conflict = original.clone();
    conflict.request.input = json!({"prompt":"different"});
    assert_eq!(create(&restarted, conflict).await.unwrap().status, 409);
    assert_eq!(std::fs::read(&marker).unwrap(), frozen);
    restarted.freeze_admission().await.unwrap();
    assert_eq!(
        create(&restarted, original.clone()).await.unwrap().status,
        503
    );
    assert_eq!(std::fs::read(&marker).unwrap(), frozen);
    restarted.reopen_admission().await.unwrap();
    let mut retry = original.clone();
    retry.index.created_at = 2;
    retry.definition = Some(json!({"must_not_replace_the_frozen_definition":true}));
    assert_eq!(create(&restarted, retry).await.unwrap().status, 200);
    let bytes =
        std::fs::read(root.path().join("node/agent/agent-prepared/execution.json")).unwrap();
    let stored: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(stored["assignment"]["index"]["created_at"], 1);
    assert_eq!(stored["assignment"]["request"], json!(original.request));
    assert_eq!(stored["assignment"]["definition"], Value::Null);
    assert!(
        !marker.exists(),
        "accepted journal now owns the frozen input"
    );
}

#[tokio::test]
async fn project_preparation_advances_only_after_explicit_rejection() {
    let root = tempfile::tempdir().unwrap();
    let _home = opencoder_core::config::scoped_config_home(root.path().join("home"));
    let worker = open(root.path()).await;
    let mut first = assignment(&worker, "project-attempts", ExecutionKind::Project);
    first.request.input = json!({"run_id":"prun-first","action":"execute"});
    super::preparation::begin(&worker, first.clone())
        .unwrap()
        .unwrap();
    let mut next = first.clone();
    next.request.input = json!({"run_id":"prun-next","action":"plan"});
    next.index.created_at = 2;
    assert_eq!(
        super::preparation::begin(&worker, next.clone())
            .unwrap()
            .unwrap_err()
            .status,
        409
    );
    super::preparation::reject_project(&worker, &first).unwrap();
    worker.inner.stopping.cancel();
    drop(worker);
    let restarted = open(root.path()).await;
    let accepted = super::preparation::begin(&restarted, next.clone())
        .unwrap()
        .unwrap();
    assert_eq!(accepted.request, next.request);
    assert_eq!(accepted.index.created_at, first.index.created_at);
    let mut third = next;
    third.request.input["run_id"] = json!("prun-third");
    assert_eq!(
        super::preparation::begin(&restarted, third)
            .unwrap()
            .unwrap_err()
            .status,
        409
    );
}

#[tokio::test]
async fn cold_retry_and_disconnected_reply_do_not_strand_host_capacity() {
    use opencoder_store::fleet::FleetStore;
    let root = tempfile::tempdir().unwrap();
    let _home = opencoder_core::config::scoped_config_home(root.path().join("home"));
    let database = root.path().join("host.db");
    let host = FleetStore::open(&database).await.unwrap();
    host.initialize_capacity(1).await.unwrap();
    std::fs::create_dir_all(root.path().join("node")).unwrap();
    std::fs::write(
        root.path().join("node/host-binding.json"),
        serde_json::to_vec(&json!({"database": database, "runtime_id": "test-runtime"})).unwrap(),
    )
    .unwrap();
    let worker = open(root.path()).await;
    let mut original = assignment(&worker, "agent-cold-retry", ExecutionKind::Agent);
    original.request.input = json!({"prompt":"one accepted request", "value":"quoted \"文本\""});
    let lifecycle = worker.lifecycle_gate(&original.index.id).await;
    let launch_blocked = lifecycle.lock().await;
    let busy = worker
        .inner
        .resource_preparations
        .acquire_many(4)
        .await
        .unwrap();
    let first_worker = worker.clone();
    let first_input = original.clone();
    let first = tokio::spawn(async move { create(&first_worker, first_input).await });
    let marker = root
        .path()
        .join("node/agent/agent-cold-retry/pending-create.json");
    tokio::time::timeout(Duration::from_secs(2), async {
        while !marker.exists() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("preparation must not wait for the execution lifecycle gate");
    let retry_worker = worker.clone();
    let mut retry_input = original.clone();
    retry_input.index.created_at = 99;
    let retry = tokio::spawn(async move { create(&retry_worker, retry_input).await });
    tokio::task::yield_now().await;
    assert!(
        !retry.is_finished(),
        "retry must share the cold preparation"
    );
    drop(busy);
    let reply = tokio::time::timeout(Duration::from_secs(5), retry)
        .await
        .expect("same-ID retry deadlocked with admission and launch")
        .unwrap()
        .unwrap();
    assert_eq!(reply.status, 200, "{reply:?}");
    assert_eq!(reply.body["created_at"], original.index.created_at);
    tokio::time::timeout(Duration::from_secs(2), async {
        while host.capacity().await.unwrap().running != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        !first.is_finished(),
        "launch must still wait for its lifecycle gate"
    );
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    assert!(
        worker.inner.admission.try_lock().is_err(),
        "disconnected RPC must leave dispatch admission with the runtime task"
    );
    drop(launch_blocked);
    let completed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let status = worker.inner.journal.lock().await.records[&original.index.id]
                .assignment
                .index
                .status;
            if status == ExecutionStatus::Idle && host.capacity().await.unwrap().running == 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    if completed.is_err() {
        let journal = worker.inner.journal.lock().await;
        let record = &journal.records[&original.index.id];
        panic!(
            "request after disconnect: {:?}, error {:?}, tickets {:?}",
            record.assignment.index.status,
            record.error,
            host.runtime_tickets("test-runtime").await.unwrap()
        );
    }
    let journal = worker.inner.journal.lock().await;
    let record = &journal.records[&original.index.id];
    assert_eq!(journal.records.len(), 1);
    assert_eq!(record.assignment.request, original.request);
    assert_eq!(
        record.assignment.index.created_at,
        original.index.created_at
    );
    assert!(host
        .runtime_tickets("test-runtime")
        .await
        .unwrap()
        .is_empty());
    assert!(!marker.exists());
}

#[tokio::test]
async fn cancelled_preflight_holds_execution_and_copy_leases_until_io_finishes() {
    use std::sync::Arc;
    use tokio::sync::{Mutex, Semaphore};
    let lifecycle = Arc::new(Mutex::new(()));
    let capacity = Arc::new(Semaphore::new(1));
    let lease = (
        lifecycle.clone().lock_owned().await,
        Some(capacity.clone().acquire_owned().await.unwrap()),
    );
    let (started, waiting) = tokio::sync::oneshot::channel();
    let (release, receiver) = std::sync::mpsc::channel();
    let call = tokio::spawn(super::preparation::run(lease, move || {
        started.send(()).unwrap();
        receiver
            .recv_timeout(std::time::Duration::from_secs(3))
            .unwrap();
    }));
    waiting.await.unwrap();
    call.abort();
    assert!(call.await.unwrap_err().is_cancelled());
    assert!(lifecycle.try_lock().is_err());
    assert_eq!(capacity.available_permits(), 0);
    release.send(()).unwrap();
    let _guard = tokio::time::timeout(std::time::Duration::from_secs(1), lifecycle.lock())
        .await
        .unwrap();
    assert_eq!(capacity.available_permits(), 1);
}
