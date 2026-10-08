#![allow(dead_code)]
mod fleet;
pub mod messages;

use anyhow::Result;
use opencoder_core::fleet::*;
use opencoder_llm::{ChatRequest, ChatStream, LlmEvent, MockChatClient};
use opencoder_node::fleet::NodeService;
use opencoder_store::{
    LibsqlStore, ProjectExecutorKind, ProjectStore, ProjectTodoRunKind, ProjectTodoRunRecord,
    ProjectTodoRunStatus,
};
use opencoder_worker::{DrainPolicy, StorageCapacity, Worker, WorkerOptions, WorkerRuntime};
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

/// Keep current-thread integration runtimes independent of host credentials,
/// harness settings and NFS agent pools. Hold both values until the test ends.
pub fn isolated_config() -> (opencoder_core::config::ScopedConfigHome, tempfile::TempDir) {
    let home = tempfile::tempdir().unwrap();
    let guard = opencoder_core::config::scoped_config_home(home.path().to_path_buf());
    (guard, home)
}

pub fn isolated_brain_config() -> (opencoder_core::config::ScopedConfigHome, tempfile::TempDir) {
    let scope = isolated_config();
    let path = opencoder_core::Config::global_config_path().unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, r#"{"context_limit":1000000}"#).unwrap();
    scope
}

pub fn mock() -> Arc<MockChatClient> {
    Arc::new(
        MockChatClient::new().with_default(vec![LlmEvent::Completed {
            text: "node-owned answer".into(),
            tool_calls: vec![],
            usage: None,
        }]),
    )
}
pub async fn worker(root: &std::path::Path, client: Arc<dyn ChatStream>) -> Worker {
    let workdir = root.join("work");
    std::fs::create_dir_all(workdir.join(".opencoder")).unwrap();
    std::fs::write(workdir.join(".opencoder/ap.json"), r#"{"mode":"off"}"#).unwrap();
    Worker::open(
        WorkerOptions {
            name: "test-node".into(),
            workdir,
            data_dir: root.join("node"),
            workflow_root: None,
            max_runs: Some(4),
            dag: true,
        },
        Some(client),
    )
    .await
    .unwrap()
}

pub async fn dag_worker(
    root: &std::path::Path,
    client: Arc<dyn ChatStream>,
) -> (
    Worker,
    native::container::ContainerFixture,
    native::model::ModelBridge,
) {
    let (container, bridge) = native::environment(root, client.clone());
    (worker(root, client).await, container, bridge)
}

pub fn drain_options(root: &std::path::Path) -> WorkerOptions {
    let workdir = root.join("work");
    std::fs::create_dir_all(workdir.join(".opencoder")).unwrap();
    std::fs::write(workdir.join(".opencoder/ap.json"), r#"{"mode":"off"}"#).unwrap();
    WorkerOptions {
        name: "drain-node".into(),
        workdir,
        data_dir: root.join("node"),
        workflow_root: None,
        max_runs: Some(2),
        dag: true,
    }
}

pub fn drain_runtime(low: Arc<AtomicBool>) -> WorkerRuntime {
    WorkerRuntime {
        drain: DrainPolicy {
            natural_grace: Duration::from_millis(20),
            cleanup_grace: Duration::from_secs(2),
        },
        health: Arc::new(move |_| {
            Ok(StorageCapacity {
                available_blocks: if low.load(Ordering::SeqCst) { 9 } else { 80 },
                total_blocks: 100,
                available_inodes: Some(80),
                total_inodes: Some(100),
            })
        }),
    }
}

pub struct InterruptClient {
    pub calls: AtomicUsize,
    pub first_release: Arc<tokio::sync::Notify>,
    pub resumed_release: Arc<tokio::sync::Notify>,
}

impl ChatStream for InterruptClient {
    fn chat_stream(&self, _request: ChatRequest) -> Result<tokio::sync::mpsc::Receiver<LlmEvent>> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let release = match call {
            0 => Some(self.first_release.clone()),
            1 => Some(self.resumed_release.clone()),
            _ => None,
        };
        let (tx, rx) = tokio::sync::mpsc::channel(2);
        tokio::spawn(async move {
            if let Some(release) = release {
                release.notified().await;
            }
            let _ = tx
                .send(LlmEvent::Completed {
                    text: "done".into(),
                    tool_calls: vec![],
                    usage: None,
                })
                .await;
        });
        Ok(rx)
    }
}

pub async fn worker_with_client(root: &std::path::Path, client: Arc<dyn ChatStream>) -> Worker {
    let workdir = root.join("work");
    std::fs::create_dir_all(workdir.join(".opencoder")).unwrap();
    std::fs::write(workdir.join(".opencoder/ap.json"), r#"{"mode":"off"}"#).unwrap();
    Worker::open(
        WorkerOptions {
            name: "test-node".into(),
            workdir,
            data_dir: root.join("node"),
            workflow_root: None,
            max_runs: Some(4),
            dag: true,
        },
        Some(client),
    )
    .await
    .unwrap()
}
pub fn assignment(
    worker: &Worker,
    id: &str,
    kind: ExecutionKind,
    input: Value,
    definition: Option<Value>,
) -> Assignment {
    let node_id = worker.registration().id;
    Assignment {
        private_context: None,
        runtime: None,
        codex: None,
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
            target: None,
            input,
            node_id: Some(node_id),
        },
        definition,
    }
}

pub fn project_snapshot(todo_id: &str) -> Value {
    json!({
        "todo": {
            "id": todo_id,
            "initiative_id": null,
            "title": "cancel project",
            "draft": "hold this plan",
            "plan_md": null,
            "status": "draft",
            "agent": "act",
            "active_session_id": null,
            "created_at": 1,
            "updated_at": 1
        },
        "goals": [],
        "milestones": []
    })
}

pub async fn seed_project_run(
    database: &std::path::Path,
    id: &str,
    todo_id: &str,
    status: ProjectTodoRunStatus,
) {
    let store = LibsqlStore::open(database).await.unwrap();
    let version = store.next_todo_version(todo_id).await.unwrap();
    store
        .create_todo_run(&ProjectTodoRunRecord {
            input_snapshot: None,
            trace_manifest: None,
            id: id.into(),
            todo_id: todo_id.into(),
            kind: ProjectTodoRunKind::Plan,
            version,
            plan_md: None,
            output_md: None,
            agent: "act".into(),
            executor_kind: ProjectExecutorKind::Agent,
            capability_id: None,
            plan_id: None,
            output_ref: None,
            session_id: None,
            status,
            started_at: 1,
            finished_at: status.is_terminal().then_some(2),
            created_at: 1,
        })
        .await
        .unwrap();
    opencoder_session::loop_registry::notify_change();
}
pub async fn settled(worker: &Worker, id: &str) -> Value {
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            let reply = worker
                .handle(NodeOperation::Inspect {
                    execution: execution_ref(worker, id).await,
                })
                .await;
            if !matches!(
                reply.body["execution"]["status"].as_str(),
                Some("pending" | "running" | "cancelling")
            ) {
                assert_eq!(reply.status, 200, "{:?}", reply);
                return reply.body;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap()
}
pub async fn prompt(worker: &Worker, id: &str, text: &str) -> RpcReply {
    worker
        .handle(NodeOperation::Command {
            execution: execution_ref(worker, id).await,
            command: ExecutionCommand {
                action: "prompt".into(),
                input: json!({"prompt":text}),
            },
        })
        .await
}

pub async fn execution_ref(worker: &Worker, id: &str) -> ExecutionRef {
    worker
        .indexes()
        .await
        .unwrap()
        .into_iter()
        .find(|index| index.id == id)
        .unwrap_or_else(|| panic!("execution index missing for {id}"))
        .execution_ref()
}

pub mod native;
// Shared helpers: individual test binaries use different subsets, so the
// re-export is intentionally wider than any single binary needs.
#[allow(unused_imports)]
pub use {
    fleet::Fleet,
    native::{run_root as dag_run, stage_binary, stage_spin_binary, stage_stdout_binary},
};
