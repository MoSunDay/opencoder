use super::*;
use opencoder_llm::{ChatRequest, ChatStream, LlmEvent, MockChatClient};
use opencoder_worker::{HostBinding, Worker, WorkerOptions};
use serde_json::json;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

#[path = "tests/forwarding.rs"]
mod forwarding;
#[path = "tests/hibernation.rs"]
mod hibernation;
#[path = "tests/maintenance.rs"]
mod maintenance;
#[path = "tests/read_reports.rs"]
mod read_reports;
#[path = "tests/result_upgrade.rs"]
mod result_upgrade;
#[path = "tests/runtime_lifecycle.rs"]
mod runtime_lifecycle;

struct HeldModel {
    entered: AtomicUsize,
    release: Arc<tokio::sync::Notify>,
}
impl ChatStream for HeldModel {
    fn chat_stream(&self, _: ChatRequest) -> Result<tokio::sync::mpsc::Receiver<LlmEvent>> {
        self.entered.fetch_add(1, Ordering::SeqCst);
        let release = self.release.clone();
        let (tx, rx) = tokio::sync::mpsc::channel(2);
        tokio::spawn(async move {
            release.notified().await;
            let _ = tx
                .send(LlmEvent::Completed {
                    text: "kept running".into(),
                    tool_calls: vec![],
                    usage: None,
                })
                .await;
        });
        Ok(rx)
    }
}

async fn runtime(
    host: &Host,
    root: &std::path::Path,
    id: &str,
    model: Arc<dyn ChatStream>,
) -> (Worker, tokio::task::JoinHandle<()>) {
    let data = root.join(id);
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("node-id"), &host.registration.id).unwrap();
    std::fs::write(
        data.join("host-binding.json"),
        serde_json::to_vec(&HostBinding {
            database: root.join("host/host.db"),
            runtime_id: id.into(),
        })
        .unwrap(),
    )
    .unwrap();
    let workdir = root.join("work");
    std::fs::create_dir_all(workdir.join(".opencoder")).unwrap();
    std::fs::write(workdir.join(".opencoder/ap.json"), r#"{"mode":"off"}"#).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    host.store.register_runtime(&opencoder_store::fleet::handoff::RuntimeRecord {
        id:id.into(),release_id:id.into(),mode:"staged".into(),config:json!({"endpoint":endpoint,"data_dir":data,"unit":format!("opencoder-runtime-{id}.service")}),
    }).await.unwrap();
    let worker = Worker::open(
        WorkerOptions {
            name: "node".into(),
            workdir,
            data_dir: data,
            workflow_root: None,
            max_runs: Some(65535),
            dag: true,
        },
        Some(model),
    )
    .await
    .unwrap();
    let app = runtime::router(Arc::new(worker.clone()), host.token.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (worker, server)
}

fn create(host: &Host, id: &str) -> NodeOperation {
    NodeOperation::Create {
        assignment: Assignment {
            private_context: None,
            runtime: None,
            codex: None,
            definition: None,
            request: CreateExecution {
                id: id.into(),
                kind: ExecutionKind::Agent,
                target: Some("act".into()),
                input: json!({"prompt":"work","harness":"opencoder","title":"release acceptance"}),
                node_id: None,
            },
            index: ExecutionIndex {
                id: id.into(),
                kind: ExecutionKind::Agent,
                node_id: host.registration.id.clone(),
                created_at: 1,
                status: ExecutionStatus::Pending,
            },
        },
    }
}

async fn wait(mut check: impl AsyncFnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !check().await {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[test]
fn host_creation_guard_releases_on_cancel_and_deduplicates_before_capacity_is_full() {
    let admissions = super::admission::Creations::default();
    let make = |id: &str| NodeOperation::Create {
        assignment: Assignment {
            private_context: None,
            runtime: None,
            codex: None,
            definition: None,
            index: ExecutionIndex {
                id: id.into(),
                kind: ExecutionKind::Agent,
                node_id: "node-test".into(),
                status: ExecutionStatus::Pending,
                created_at: 1,
            },
            request: CreateExecution {
                id: id.into(),
                kind: ExecutionKind::Agent,
                target: Some("act".into()),
                input: json!({}),
                node_id: None,
            },
        },
    };
    let request = make("agent-duplicate");
    let lease = admissions.begin("legacy", &request).unwrap();
    assert_eq!(
        admissions.begin("legacy", &request).err().unwrap().status,
        503
    );
    assert!(admissions.begin("current", &request).is_ok());
    drop(lease);
    assert!(admissions.begin("legacy", &request).is_ok());
    assert_eq!(
        super::admission::request_timeout(&request),
        Duration::from_secs(45)
    );
}
