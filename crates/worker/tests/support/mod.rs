#![allow(dead_code)]
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

pub fn mock() -> Arc<MockChatClient> {
    Arc::new(
        MockChatClient::new().with_default(vec![LlmEvent::Completed {
            text: "node-owned answer".into(),
            tool_calls: vec![],
            usage: None,
        }]),
    )
}
pub async fn worker(root: &std::path::Path, client: Arc<MockChatClient>) -> Worker {
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
                available_blocks: if low.load(Ordering::SeqCst) { 19 } else { 80 },
                total_blocks: 100,
                available_inodes: 80,
                total_inodes: 100,
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
            "milestone_id": null,
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

async fn execution_ref(worker: &Worker, id: &str) -> ExecutionRef {
    worker
        .indexes()
        .await
        .unwrap()
        .into_iter()
        .find(|index| index.id == id)
        .unwrap_or_else(|| panic!("execution index missing for {id}"))
        .execution_ref()
}

pub struct Fleet {
    pub state: Arc<opencoder_control::AppState>,
    pub nodes: Vec<Worker>,
    app: axum::Router,
    server: tokio::task::JoinHandle<()>,
    channels: Vec<tokio::task::JoinHandle<anyhow::Result<()>>>,
    _dir: tempfile::TempDir,
}
impl Fleet {
    pub async fn new(count: usize, client: Arc<MockChatClient>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let state = opencoder_control::new_state(
            dir.path().join("server-work"),
            dir.path().join("server"),
            Some(client.clone()),
        )
        .await
        .unwrap();
        let app = opencoder_control::build_app(state.clone(), None, false);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let serve_app = app.clone();
        let server = tokio::spawn(async move {
            axum::serve(listener, serve_app).await.unwrap();
        });
        let mut nodes = vec![];
        let mut channels = vec![];
        for i in 0..count {
            let node = worker(&dir.path().join(format!("n{i}")), client.clone()).await;
            let service: Arc<dyn NodeService> = Arc::new(node.clone());
            let url = url.clone();
            channels.push(tokio::spawn(async move {
                opencoder_node::fleet::run(&url, "test", service).await
            }));
            nodes.push(node);
        }
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            // Scheduling (select_node) requires a snapshot with ready=true;
            // waiting for online alone races the first submit into 503.
            while state
                .hub
                .views()
                .await
                .iter()
                .filter(|n| n.online && n.snapshot.as_ref().is_some_and(|s| s.ready))
                .count()
                != count
            {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        Self {
            state,
            nodes,
            app,
            server,
            channels,
            _dir: dir,
        }
    }
    pub async fn call(&self, method: &str, path: &str, body: Value) -> RpcReply {
        use tower::ServiceExt;
        let body = if body.is_null() {
            String::new()
        } else {
            body.to_string()
        };
        let response = self
            .app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method(method)
                    .uri(path)
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status().as_u16();
        let bytes = axum::body::to_bytes(response.into_body(), MAX_FRAME_BYTES)
            .await
            .unwrap();
        RpcReply {
            status,
            body: serde_json::from_slice(&bytes).unwrap(),
        }
    }
    pub async fn response(&self, method: &str, path: &str) -> axum::response::Response {
        use tower::ServiceExt;
        self.app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method(method)
                    .uri(path)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
    }
    pub fn root(&self) -> &std::path::Path {
        self._dir.path()
    }
    pub async fn shutdown(&self) {
        for node in &self.nodes {
            node.shutdown().await.unwrap();
        }
    }
    pub async fn disconnect(&self, index: usize) {
        self.channels[index].abort();
        let node_id = self.nodes[index].registration().id;
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if self
                    .state
                    .hub
                    .views()
                    .await
                    .iter()
                    .find(|node| node.registration.id == node_id)
                    .is_some_and(|node| !node.online)
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }
}
impl Drop for Fleet {
    fn drop(&mut self) {
        for channel in &self.channels {
            channel.abort();
        }
        self.server.abort();
    }
}

mod wasm;
// Shared helpers: individual test binaries use different subsets, so the
// re-export is intentionally wider than any single binary needs.
#[allow(unused_imports)]
pub use wasm::{stage_spin_wasm, stage_stdout_wasm};
