use super::*;
use opencoder_llm::{ChatRequest, ChatStream, LlmEvent, MockChatClient};
use opencoder_worker::{HostBinding, Worker, WorkerOptions};
use serde_json::json;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

#[path = "tests/hibernation.rs"]
mod hibernation;
#[path = "tests/read_reports.rs"]
mod read_reports;

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

async fn assert_stalled_forward_releases_its_lock(creation: bool) {
    let dir = tempfile::tempdir().unwrap();
    let host = Host::open(
        &dir.path().join("host"),
        "node".into(),
        "test-token".into(),
        1,
    )
    .await
    .unwrap();
    let entered = Arc::new(tokio::sync::Notify::new());
    let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let seen = requests.clone();
    let started = entered.clone();
    let app = axum::Router::new().route(
        "/rpc",
        axum::routing::post(move |axum::Json(operation): axum::Json<NodeOperation>| {
            let seen = seen.clone();
            let started = started.clone();
            async move {
                let first = {
                    let mut requests = seen.lock().unwrap();
                    requests.push(serde_json::to_value(&operation).unwrap());
                    requests.len() == 1
                };
                if first {
                    started.notify_one();
                    std::future::pending::<()>().await;
                }
                let body = match operation {
                    NodeOperation::Create { assignment } => json!(assignment.index),
                    NodeOperation::Inspect { execution } => json!({"id":execution.id}),
                    _ => panic!("unexpected operation"),
                };
                axum::Json(RpcReply::ok(body))
            }
        }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    host.store
        .register_runtime(&opencoder_store::fleet::handoff::RuntimeRecord {
            id: "slow-runtime".into(),
            release_id: "slow-release".into(),
            mode: "staged".into(),
            config: json!({"endpoint":endpoint,"data_dir":dir.path().join("runtime"),
                "unit":"opencoder-runtime-slow.service"}),
        })
        .await
        .unwrap();
    host.store.activate_runtime("slow-runtime").await.unwrap();
    let operation = if creation {
        create(&host, "agent-slow-forward")
    } else {
        host.store
            .assign_runtime("agent-slow-forward", None)
            .await
            .unwrap();
        NodeOperation::Inspect {
            execution: ExecutionRef {
                id: "agent-slow-forward".into(),
                kind: ExecutionKind::Agent,
            },
        }
    };
    let waiting = host.clone();
    let original = operation.clone();
    let call = tokio::spawn(async move { waiting.handle(original).await });
    entered.notified().await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(if creation { 46 } else { 11 })).await;
    let reply = call.await.unwrap();
    tokio::time::resume();
    assert_eq!(reply.status, 504, "{reply:?}");
    assert!(reply.body["error"]
        .as_str()
        .unwrap()
        .contains("runtime request timed out"));
    let lock = tokio::time::timeout(
        Duration::from_secs(1),
        host.store.request_lock("runtime-use", "slow-runtime"),
    )
    .await
    .expect("timed-out forwarding retained the shared runtime lock")
    .unwrap();
    drop(lock);
    assert_eq!(
        host.store
            .owner("agent-slow-forward")
            .await
            .unwrap()
            .unwrap()
            .runtime_id,
        "slow-runtime"
    );
    assert_eq!(host.handle(operation).await.status, 200);
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0], requests[1], "retry changed the frozen request");
    server.abort();
}

#[tokio::test]
async fn stalled_runtime_create_releases_its_lock_and_preserves_retry_ownership() {
    assert_stalled_forward_releases_its_lock(true).await;
}

#[tokio::test]
async fn stalled_runtime_inspection_releases_its_lock_before_server_read_timeout() {
    assert_stalled_forward_releases_its_lock(false).await;
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

#[tokio::test]
async fn host_dialogs_clear_deletes_live_runtime_and_trims_hibernated_inventory() {
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
    let fast = Arc::new(
        MockChatClient::new().with_default(vec![LlmEvent::Completed {
            text: "operator done".into(),
            tool_calls: vec![],
            usage: None,
        }]),
    );
    let (live, _live_http) = runtime(&host, dir.path(), "r-live", fast.clone()).await;
    host.store.activate_runtime("r-live").await.unwrap();

    // A retired-but-hibernated runtime is only represented by its saved
    // final inventory; it holds one droppable and one live operator row.
    host.store
        .register_runtime(&opencoder_store::fleet::handoff::RuntimeRecord {
            id: "r-sleep".into(),
            release_id: "r-sleep".into(),
            mode: "staged".into(),
            config: json!({"endpoint":"http://127.0.0.1:1","data_dir":dir.path().join("r-sleep"),"unit":"opencoder-runtime-r-sleep.service"}),
        })
        .await
        .unwrap();
    // Retire r-live's predecessor slot by promoting r-sleep then re-activating
    // the real one, leaving r-sleep retired (and hibernated) in the catalog.
    host.store.activate_runtime("r-sleep").await.unwrap();
    host.store.activate_runtime("r-live").await.unwrap();
    let saved = json!({
        "registration": {"id": host.registration.id},
        "snapshot": {"ready": false},
        "indexes": [
            {"id":"operator-done-1","created_at":1,"kind":"operator","node_id":host.registration.id,"status":"idle"},
            {"id":"agent-done-1","created_at":1,"kind":"agent","node_id":host.registration.id,"status":"idle"},
            {"id":"operator-live-1","created_at":2,"kind":"operator","node_id":host.registration.id,"status":"interrupted"}
        ]
    });
    host.store
        .put_definition("runtime_sleep", "r-sleep", &saved)
        .await
        .unwrap();

    // One live operator execution on the active runtime.
    let reply = host
        .handle(NodeOperation::Create {
            assignment: Assignment {
                private_context: None,
                runtime: None,
                codex: None,
                definition: None,
                request: CreateExecution {
                    id: "operator-done-2".into(),
                    kind: ExecutionKind::Operator,
                    target: None,
                    input: json!({"prompt":"hi"}),
                    node_id: Some(host.registration.id.clone()),
                },
                index: ExecutionIndex {
                    id: "operator-done-2".into(),
                    kind: ExecutionKind::Operator,
                    node_id: host.registration.id.clone(),
                    created_at: 1,
                    status: ExecutionStatus::Pending,
                },
            },
        })
        .await;
    assert_eq!(reply.status, 200, "{:?}", reply);
    let journal = dir
        .path()
        .join("r-live/operator/operator-done-2/execution.json");
    wait(async || journal.is_file()).await;
    wait(async || {
        let inspect = live
            .indexes()
            .await
            .unwrap()
            .into_iter()
            .find(|i| i.id == "operator-done-2")
            .map(|i| i.status);
        inspect == Some(ExecutionStatus::Idle)
    })
    .await;

    let maintenance = |input: serde_json::Value| NodeOperation::Maintenance {
        command: ExecutionCommand {
            action: "dialogs_clear".into(),
            input,
        },
    };
    let reply = host
        .handle(maintenance(json!({"sessions": ["operator-done-1", "operator-done-2", "operator-live-1", "operator-ghost"]})))
        .await;
    assert_eq!(reply.status, 200, "{:?}", reply);
    assert_eq!(reply.body["removed"], json!(2));
    assert_eq!(reply.body["forgotten"], json!(1));
    assert_eq!(reply.body["skipped"], json!(["operator-live-1"]));

    // The live runtime lost its session and journal record.
    assert!(!journal.exists());
    // The hibernated inventory kept the live row and dropped the droppable one.
    let kept = host
        .store
        .definition("runtime_sleep", "r-sleep")
        .await
        .unwrap()
        .unwrap();
    let ids: Vec<String> = kept["indexes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|i| i["id"].as_str().map(str::to_owned))
        .collect();
    assert_eq!(
        ids,
        vec!["agent-done-1".to_string(), "operator-live-1".to_string()]
    );
    // A second lane clear is independent and can remove the hibernated Agent
    // row without touching the surviving Operator record.
    let reply = host
        .handle(maintenance(json!({
            "kind": "agent",
            "sessions": ["agent-done-1", "operator-live-1"]
        })))
        .await;
    assert_eq!(reply.status, 200, "{:?}", reply);
    assert_eq!(reply.body["kind"], json!("agent"));
    assert_eq!(reply.body["removed"], json!(1));
    assert_eq!(reply.body["skipped"], json!([]));
    let kept = host
        .store
        .definition("runtime_sleep", "r-sleep")
        .await
        .unwrap()
        .unwrap();
    let ids: Vec<String> = kept["indexes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|i| i["id"].as_str().map(str::to_owned))
        .collect();
    assert_eq!(ids, vec!["operator-live-1".to_string()]);
    live.shutdown().await.unwrap();
}

#[tokio::test]
async fn stalled_legacy_creates_are_bounded_and_cannot_fill_the_host_channel() {
    let dir = tempfile::tempdir().unwrap();
    let _scope = opencoder_core::config::scoped_config_home(dir.path().join("home"));
    let host = Host::open(
        &dir.path().join("host"),
        "bounded".into(),
        "test".into(),
        20,
    )
    .await
    .unwrap();
    let (entered, mut requests) = tokio::sync::mpsc::channel(8);
    let old_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let old_endpoint = format!("http://{}", old_listener.local_addr().unwrap());
    let slow = axum::Router::new().route(
        "/rpc",
        axum::routing::post(move |axum::Json(operation): axum::Json<NodeOperation>| {
            let entered = entered.clone();
            async move {
                if matches!(&operation, NodeOperation::Create { assignment }
                    if !opencoder_worker::requires_agent_pool(assignment))
                {
                    return axum::Json(RpcReply::ok(json!({"accepted":true})));
                }
                entered.send(()).await.unwrap();
                std::future::pending::<axum::Json<RpcReply>>().await
            }
        }),
    );
    let slow_server = tokio::spawn(async move { axum::serve(old_listener, slow).await.unwrap() });
    let new_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let new_endpoint = format!("http://{}", new_listener.local_addr().unwrap());
    let fast = axum::Router::new().route(
        "/rpc",
        axum::routing::post(|| async { axum::Json(RpcReply::ok(json!({"accepted":true}))) }),
    );
    let fast_server = tokio::spawn(async move { axum::serve(new_listener, fast).await.unwrap() });
    for (id, endpoint) in [("legacy", old_endpoint), ("current", new_endpoint)] {
        host.store
            .register_runtime(&opencoder_store::fleet::handoff::RuntimeRecord {
                id: id.into(),
                release_id: id.into(),
                mode: "staged".into(),
                config: json!({"endpoint":endpoint,"data_dir":dir.path().join(id),
                "unit":format!("opencoder-runtime-{id}.service")}),
            })
            .await
            .unwrap();
    }
    let mut pending = Vec::new();
    for index in 0..4 {
        let owner = host.clone();
        let operation = create(&host, &format!("agent-held-{index}"));
        pending.push(tokio::spawn(async move {
            owner.call_runtime("legacy", &operation).await.unwrap()
        }));
        tokio::time::timeout(Duration::from_secs(2), requests.recv())
            .await
            .unwrap()
            .unwrap();
    }
    // The old version never acknowledges Create. Repeated submissions must
    // return promptly rather than occupy all 128 fleet RPC permits.
    for index in 0..128 {
        let operation = create(&host, &format!("agent-extra-{index}"));
        let reply = tokio::time::timeout(
            Duration::from_secs(1),
            host.call_runtime("legacy", &operation),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(reply.status, 503);
    }
    assert_eq!(
        host.call_runtime("legacy", &create(&host, "agent-held-0"))
            .await
            .unwrap()
            .status,
        503
    );
    let mut wasi = create(&host, "dag-independent-wasi");
    if let NodeOperation::Create { assignment } = &mut wasi {
        assignment.index.kind = ExecutionKind::Dag;
        assignment.request.kind = ExecutionKind::Dag;
        assignment.request.target = None;
        assignment.request.input = json!({});
        assignment.definition = Some(json!({"name":"independent","steps":[{
            "name":"run","kind":{"type":"wasm","command":"quick.wasm"}}]}));
    }
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), host.call_runtime("legacy", &wasi))
            .await
            .unwrap()
            .unwrap()
            .status,
        200,
        "pure WASI admission waited for unrelated cold resource copies"
    );
    assert_eq!(
        host.call_runtime("current", &create(&host, "agent-current"))
            .await
            .unwrap()
            .status,
        200
    );
    assert_eq!(
        host.call_runtime("current", &create(&host, "agent-current"))
            .await
            .unwrap()
            .status,
        200
    );
    let inspect = NodeOperation::Inspect {
        execution: ExecutionRef {
            id: "agent-held-0".into(),
            kind: ExecutionKind::Agent,
        },
    };
    let owner = host.clone();
    let query = tokio::spawn(async move { owner.call_runtime("legacy", &inspect).await.unwrap() });
    tokio::time::timeout(Duration::from_secs(2), requests.recv())
        .await
        .unwrap()
        .unwrap();
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(16)).await;
    assert_eq!(
        query.await.unwrap().status,
        504,
        "read RPC must have a bounded lifetime"
    );
    assert!(
        pending.iter().all(|task| !task.is_finished()),
        "cold creation retains its larger budget"
    );
    tokio::time::advance(Duration::from_secs(45)).await;
    for task in pending {
        assert_eq!(task.await.unwrap().status, 504);
    }
    let retry = create(&host, "agent-held-0");
    assert!(
        host.creations.begin("legacy", &retry).is_ok(),
        "timeout leaked creation capacity"
    );
    slow_server.abort();
    fast_server.abort();
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
