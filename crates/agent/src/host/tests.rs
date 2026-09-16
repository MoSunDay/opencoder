use super::*;
use opencoder_llm::{ChatRequest, ChatStream, LlmEvent, MockChatClient};
use opencoder_worker::{HostBinding, Worker, WorkerOptions};
use serde_json::json;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

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

#[tokio::test]
async fn three_runtime_versions_keep_live_model_calls_and_global_fifo() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_test_writer()
        .try_init();
    let dir = tempfile::tempdir().unwrap();
    let _scope = opencoder_core::config::scoped_config_home(dir.path().join("home"));
    let host = Host::open(
        &dir.path().join("host"),
        "node".into(),
        "test-token".into(),
        1,
    )
    .await
    .unwrap();
    let held = Arc::new(HeldModel {
        entered: AtomicUsize::new(0),
        release: Arc::new(tokio::sync::Notify::new()),
    });
    let fast = Arc::new(
        MockChatClient::new().with_default(vec![LlmEvent::Completed {
            text: "new release".into(),
            tool_calls: vec![],
            usage: None,
        }]),
    );
    let (old, old_http) = runtime(&host, dir.path(), "r1", held.clone()).await;
    let (new, new_http) = runtime(&host, dir.path(), "r2", fast.clone()).await;
    let (latest, latest_http) = runtime(&host, dir.path(), "r3", fast.clone()).await;
    host.store.activate_runtime("r1").await.unwrap();
    assert_eq!(host.handle(create(&host, "agent-long")).await.status, 200);
    wait(async || held.entered.load(Ordering::SeqCst) == 1).await;
    host.store.activate_runtime("r2").await.unwrap();
    assert_eq!(host.handle(create(&host, "agent-next")).await.status, 200);
    host.store.activate_runtime("r3").await.unwrap();
    assert_eq!(host.handle(create(&host, "agent-latest")).await.status, 200);
    assert_eq!(host.store.capacity().await.unwrap().running, 1);
    assert_eq!(host.store.capacity().await.unwrap().queued, 2);
    assert_eq!(held.entered.load(Ordering::SeqCst), 1);
    assert_eq!(fast.call_count(), 0);
    let replay = host.handle(create(&host, "agent-long")).await;
    assert_eq!(replay.status, 200);
    assert_eq!(
        host.store
            .owner("agent-long")
            .await
            .unwrap()
            .unwrap()
            .runtime_id,
        "r1"
    );
    assert!(!old.can_hibernate().await);
    held.release.notify_one();
    wait(async || {
        host.store.capacity().await.unwrap().queued == 0
            && host.store.capacity().await.unwrap().running == 0
    })
    .await;
    assert_eq!(held.entered.load(Ordering::SeqCst), 1);
    assert_eq!(fast.call_count(), 2);
    assert_eq!(
        host.store
            .owner("agent-next")
            .await
            .unwrap()
            .unwrap()
            .runtime_id,
        "r2"
    );
    let detail = host
        .handle(NodeOperation::Inspect {
            execution: ExecutionRef {
                id: "agent-long".into(),
                kind: ExecutionKind::Agent,
            },
        })
        .await;
    assert_eq!(detail.status, 200);
    assert_eq!(detail.body["execution"]["status"], "idle");
    old_http.abort();
    new_http.abort();
    latest_http.abort();
    for worker in [old, new, latest] {
        worker.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn deployment_http_requires_authentication_current_host_and_current_server() {
    let dir = tempfile::tempdir().unwrap();
    let host = Host::open(
        &dir.path().join("host"),
        "node".into(),
        "signal-fixture".into(),
        1,
    )
    .await
    .unwrap();
    host.store
        .put_definition("host", "current", &json!({"instance":host.instance}))
        .await
        .unwrap();
    std::fs::write(
        dir.path().join("host/deployment.json"),
        serde_json::to_vec(&json!({
        "state_dir":dir.path(),"unit_prefix":"opencoder-release-fixture"}))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("release-state.json"),
        br#"{"current":"r2"}"#,
    )
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!(
        "http://{}/deployment-signal",
        listener.local_addr().unwrap()
    );
    let app = api::router(host.clone());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    assert_eq!(
        client
            .post(&endpoint)
            .json(&json!({"action":"deploy","release_id":"r2"}))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    for (body, expected) in [
        (json!({"action":"deploy","release_id":"r1"}), "superseded"),
        (
            json!({"action":"restart","release_id":"r2"}),
            "unknown release action",
        ),
    ] {
        let reply = client
            .post(&endpoint)
            .bearer_auth("signal-fixture")
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(reply.status(), 409);
        assert!(reply.text().await.unwrap().contains(expected));
    }
    host.store
        .put_definition("host", "current", &json!({"instance":"replacement"}))
        .await
        .unwrap();
    let reply = client
        .post(&endpoint)
        .bearer_auth("signal-fixture")
        .json(&json!({"action":"deploy","release_id":"r2"}))
        .send()
        .await
        .unwrap();
    assert_eq!(reply.status(), 409);
    assert!(reply.text().await.unwrap().contains("Host is not current"));
    task.abort();
}

#[tokio::test]
async fn host_scheduling_reports_workdir_unsupported_and_rejects_workdir_with_400() {
    let dir = tempfile::tempdir().unwrap();
    let _scope = opencoder_core::config::scoped_config_home(dir.path().join("home"));
    let host = Host::open(
        &dir.path().join("host"),
        "node".into(),
        "test-token".into(),
        1,
    )
    .await
    .unwrap();
    let maintenance = |action: &str, input: serde_json::Value| NodeOperation::Maintenance {
        command: ExecutionCommand {
            action: action.into(),
            input,
        },
    };
    // The read endpoint exposes the capability flag so the UI can hide the
    // workdir input instead of submitting a value the host must reject.
    let reply = host
        .handle(maintenance("scheduling", serde_json::Value::Null))
        .await;
    assert_eq!(reply.status, 200);
    assert_eq!(reply.body["queue_order"], "fifo");
    assert_eq!(reply.body["workdir"], serde_json::Value::Null);
    assert_eq!(reply.body["workdir_supported"], false);
    // Capability violations are 400 client errors, never 503 "host routing".
    let reply = host
        .handle(maintenance(
            "configure_scheduling",
            json!({"max_runs":4,"queue_order":"fifo","workdir":"/data/x"}),
        ))
        .await;
    assert_eq!(reply.status, 400);
    let message = reply.body["error"].as_str().unwrap();
    assert!(message.contains("multi-runtime hosts do not support a scheduling workdir"));
    assert!(!message.contains("host routing"));
    let reply = host
        .handle(maintenance(
            "configure_scheduling",
            json!({"max_runs":4,"queue_order":"lifo"}),
        ))
        .await;
    assert_eq!(reply.status, 400);
    assert_eq!(
        reply.body["error"],
        "multi-runtime hosts require FIFO ordering"
    );
}
