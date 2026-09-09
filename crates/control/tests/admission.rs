use opencoder_control::admission::AdmissionMode;
use opencoder_core::fleet::*;
use opencoder_node::fleet::NodeService;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    Arc,
};

struct Service {
    open: AtomicBool,
    commands: AtomicUsize,
    freezes: AtomicUsize,
    freeze_delay_ms: AtomicU64,
    accepted: std::sync::Mutex<HashMap<String, Value>>,
    revision: tokio::sync::watch::Sender<u64>,
}

impl Service {
    fn new() -> Arc<Self> {
        let (revision, _) = tokio::sync::watch::channel(0);
        Arc::new(Self {
            open: AtomicBool::new(true),
            commands: AtomicUsize::new(0),
            freezes: AtomicUsize::new(0),
            freeze_delay_ms: AtomicU64::new(0),
            accepted: std::sync::Mutex::new(HashMap::new()),
            revision,
        })
    }
}

#[async_trait::async_trait]
impl NodeService for Service {
    fn registration(&self) -> NodeRegistration {
        NodeRegistration {
            protocol_version: PROTOCOL_VERSION,
            id: "node-a".into(),
            name: "node-a".into(),
            version: "test".into(),
            maintenance_agent_id: "act".into(),
            kinds: vec![ExecutionKind::Agent],
        }
    }

    fn snapshot(&self) -> NodeSnapshot {
        NodeSnapshot {
            pending_runs: 0,
            queue_order: Default::default(),
            generation: "generation-a".into(),
            sequence: 1,
            cpu_capacity: 2.0,
            active_agent_loops: 0,
            active_runs: 0,
            max_runs: 2,
            ready: self.open.load(Ordering::SeqCst),
            resource_error: (!self.open.load(Ordering::SeqCst))
                .then(|| "node admission is frozen".into()),
        }
    }

    fn changes(&self) -> tokio::sync::watch::Receiver<u64> {
        self.revision.subscribe()
    }

    async fn indexes(&self) -> anyhow::Result<Vec<ExecutionIndex>> {
        Ok(Vec::new())
    }

    async fn handle(&self, operation: NodeOperation) -> RpcReply {
        match operation {
            NodeOperation::Admission { command } => {
                match command {
                    NodeAdmissionCommand::Freeze => {
                        self.freezes.fetch_add(1, Ordering::SeqCst);
                        tokio::time::sleep(std::time::Duration::from_millis(
                            self.freeze_delay_ms.load(Ordering::SeqCst),
                        ))
                        .await;
                        self.open.store(false, Ordering::SeqCst);
                    }
                    NodeAdmissionCommand::Reopen => self.open.store(true, Ordering::SeqCst),
                    NodeAdmissionCommand::Status => {}
                }
                RpcReply::ok(json!({
                    "mode": if self.open.load(Ordering::SeqCst) { "open" } else { "frozen" },
                    "active_runs": 0,
                    "owned_processes": 0,
                }))
            }
            NodeOperation::Create { assignment } => {
                if let Some(receipt) = assignment.request.input.get("brain_receipt") {
                    self.accepted.lock().unwrap().insert(
                        assignment.index.id.clone(),
                        json!({"id":assignment.index.id,"kind":assignment.index.kind,"receipt":receipt}),
                    );
                }
                RpcReply::ok(json!(assignment.index))
            }
            NodeOperation::AcceptedRequest { execution } => self
                .accepted
                .lock()
                .unwrap()
                .get(&execution.id)
                .cloned()
                .map(RpcReply::ok)
                .unwrap_or_else(|| RpcReply::error(404, "accepted request not found")),
            NodeOperation::Command { .. } => {
                self.commands.fetch_add(1, Ordering::SeqCst);
                RpcReply::ok(json!({"ok":true}))
            }
            _ => RpcReply::ok(Value::Null),
        }
    }
}

struct Harness {
    base: String,
    state: Arc<opencoder_control::AppState>,
    service: Arc<Service>,
    server: tokio::task::JoinHandle<()>,
    node: tokio::task::JoinHandle<anyhow::Result<()>>,
    _dir: tempfile::TempDir,
}

impl Harness {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let state =
            opencoder_control::new_state(dir.path().join("work"), dir.path().join("server"), None)
                .await
                .unwrap();
        let app = opencoder_control::build_app(state.clone(), None, false);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let service = Service::new();
        let remote = base.clone();
        let node_service: Arc<dyn NodeService> = service.clone();
        let node = tokio::spawn(async move {
            opencoder_node::fleet::run(&remote, "test-token", node_service).await
        });
        tokio::time::timeout(std::time::Duration::from_secs(20), async {
            loop {
                assert!(
                    !node.is_finished(),
                    "node channel exited before initial sync"
                );
                if state.hub.views().await.iter().any(|node| {
                    node.online
                        && node
                            .snapshot
                            .as_ref()
                            .is_some_and(|snapshot| snapshot.ready)
                }) {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        Self {
            base,
            state,
            service,
            server,
            node,
            _dir: dir,
        }
    }

    async fn request(&self, method: reqwest::Method, path: &str, body: Value) -> reqwest::Response {
        reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .request(method, format!("{}{}", self.base, path))
            .json(&body)
            .send()
            .await
            .unwrap()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.node.abort();
        self.server.abort();
    }
}

#[tokio::test]
async fn freeze_blocks_new_inputs_but_keeps_read_stop_and_question_answers() {
    let harness = Harness::new().await;
    let index = ExecutionIndex {
        id: "agent-existing".into(),
        created_at: 1,
        kind: ExecutionKind::Agent,
        node_id: "node-a".into(),
        status: ExecutionStatus::Idle,
    };
    harness.state.fleet.put_index(&index).await.unwrap();
    #[derive(serde::Serialize)]
    struct CanonicalIntent<'a> {
        situation: &'a str,
        node_id: Option<&'a str>,
        plan_id: Option<&'a str>,
        top_k: u32,
        replan: bool,
        model: Option<&'a str>,
    }
    let canonical = CanonicalIntent {
        situation: "existing work",
        node_id: None,
        plan_id: None,
        top_k: 10,
        replan: false,
        model: None,
    };
    let fingerprint =
        opencoder_brain::situation_digest(&serde_json::to_string(&canonical).unwrap());
    let intent = serde_json::to_value(canonical).unwrap();
    let brain_index = ExecutionIndex {
        id: "agent-frozen-retry".into(),
        created_at: 2,
        kind: ExecutionKind::Agent,
        node_id: "node-a".into(),
        status: ExecutionStatus::Idle,
    };
    harness.state.fleet.put_index(&brain_index).await.unwrap();
    harness.service.accepted.lock().unwrap().insert(
        brain_index.id.clone(),
        json!({
            "id":brain_index.id,"kind":"agent","receipt":{
                "version":1,"request_id":"frozen-retry","fingerprint":fingerprint,"intent":intent,
                "decision":{"plan_id":"plan-a","capability_id":"cap-a","reason":null,"path":[],
                    "planned_fresh":false,"planner_model":"fixture","kind":"agent","target":"act"}
            }
        }),
    );

    let frozen = harness
        .request(reqwest::Method::POST, "/api/admin/drain", json!({}))
        .await;
    assert_eq!(frozen.status(), 200);
    assert!(!harness.service.open.load(Ordering::SeqCst));
    assert_eq!(
        harness
            .request(reqwest::Method::GET, "/api/ready", json!({}))
            .await
            .status(),
        503
    );

    for (path, body) in [
        (
            "/api/executions",
            json!({"id":"agent-new","kind":"agent","input":{"prompt":"new"}}),
        ),
        (
            "/api/executions/agent-existing/commands",
            json!({"action":"prompt","input":{"prompt":"continue"}}),
        ),
        (
            "/api/sessions/agent-existing/prompt?delivery=queue",
            json!({"prompt":"later"}),
        ),
        ("/api/brain/preview", json!({"situation":"new planning"})),
    ] {
        let reply = harness.request(reqwest::Method::POST, path, body).await;
        assert_eq!(reply.status(), 503, "{path}");
    }
    assert_eq!(harness.service.commands.load(Ordering::SeqCst), 0);
    assert_eq!(
        harness
            .request(
                reqwest::Method::POST,
                "/api/brain/dispatch",
                json!({"situation":"existing work","request_id":"frozen-retry"}),
            )
            .await
            .status(),
        202,
        "an exact existing Brain receipt retry is a read of the original decision"
    );

    assert_eq!(
        harness
            .request(
                reqwest::Method::GET,
                "/api/executions/agent-existing",
                json!({}),
            )
            .await
            .status(),
        200
    );
    for path in [
        "/api/executions/agent-existing/commands",
        "/api/sessions/agent-existing/questions/call-1/answer",
    ] {
        let body = if path.ends_with("commands") {
            json!({"action":"cancel","input":{}})
        } else {
            json!({"answer":"continue"})
        };
        assert_eq!(
            harness
                .request(reqwest::Method::POST, path, body)
                .await
                .status(),
            200,
            "{path}"
        );
    }
    assert_eq!(harness.service.commands.load(Ordering::SeqCst), 2);

    let reopened = harness
        .request(reqwest::Method::DELETE, "/api/admin/drain", json!({}))
        .await;
    assert_eq!(reopened.status(), 200);
    assert!(harness.service.open.load(Ordering::SeqCst));
}

#[tokio::test]
async fn reconnect_freeze_cannot_arrive_after_concurrent_reopen() {
    let harness = Harness::new().await;
    assert_eq!(
        harness
            .request(reqwest::Method::POST, "/api/admin/drain", json!({}))
            .await
            .status(),
        200
    );
    harness.node.abort();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while harness.state.hub.views().await[0].online {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let offline_reopen = harness
        .request(reqwest::Method::DELETE, "/api/admin/drain", json!({}))
        .await;
    assert_eq!(offline_reopen.status(), 503);
    assert_eq!(
        harness.state.admission.snapshot().await.mode,
        AdmissionMode::Frozen
    );

    harness.service.open.store(true, Ordering::SeqCst);
    harness.service.freeze_delay_ms.store(250, Ordering::SeqCst);
    let remote = harness.base.clone();
    let service: Arc<dyn NodeService> = harness.service.clone();
    let reconnected =
        tokio::spawn(
            async move { opencoder_node::fleet::run(&remote, "test-token", service).await },
        );
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while harness.service.freezes.load(Ordering::SeqCst) < 2 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let reopened = harness
        .request(reqwest::Method::DELETE, "/api/admin/drain", json!({}))
        .await;
    assert_eq!(reopened.status(), 200);
    assert!(harness.service.open.load(Ordering::SeqCst));
    assert_eq!(
        harness.state.admission.snapshot().await.mode,
        AdmissionMode::Open
    );
    reconnected.abort();
}
