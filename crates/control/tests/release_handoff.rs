//! Real HTTP/WebSocket handoff against two independent control databases
//! connections. The node double counts business starts, not transport retries.
use opencoder_core::fleet::*;
use opencoder_node::fleet::NodeService;
use serde_json::json;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

struct Node {
    records: Mutex<HashMap<String, Assignment>>,
    starts: AtomicUsize,
    freezes: AtomicUsize,
    sequence: AtomicU64,
    revision: tokio::sync::watch::Sender<u64>,
}
impl Node {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            records: Mutex::new(HashMap::new()),
            starts: AtomicUsize::new(0),
            freezes: AtomicUsize::new(0),
            sequence: AtomicU64::new(0),
            revision: tokio::sync::watch::channel(0).0,
        })
    }
}
#[async_trait::async_trait]
impl NodeService for Node {
    fn registration(&self) -> NodeRegistration {
        NodeRegistration {
            id: "node-release".into(),
            name: "test".into(),
            version: "test".into(),
            maintenance_agent_id: "maintainer-node-release".into(),
            protocol_version: PROTOCOL_VERSION,
            kinds: vec![ExecutionKind::Agent, ExecutionKind::Brain],
        }
    }
    fn snapshot(&self) -> NodeSnapshot {
        NodeSnapshot {
            generation: "host-00000000000000000001-test".into(),
            sequence: self.sequence.fetch_add(1, Ordering::SeqCst) + 1,
            cpu_capacity: 4.0,
            active_agent_loops: 0,
            active_runs: 0,
            pending_runs: 0,
            max_runs: 4,
            queue_order: QueueOrder::Fifo,
            ready: true,
            resource_error: None,
        }
    }
    fn changes(&self) -> tokio::sync::watch::Receiver<u64> {
        self.revision.subscribe()
    }
    async fn indexes(&self) -> anyhow::Result<Vec<ExecutionIndex>> {
        Ok(self
            .records
            .lock()
            .unwrap()
            .values()
            .map(|a| a.index.clone())
            .collect())
    }
    async fn handle(&self, operation: NodeOperation) -> RpcReply {
        match operation {
            NodeOperation::Create { assignment } => {
                let mut records = self.records.lock().unwrap();
                let entry = records
                    .entry(assignment.index.id.clone())
                    .or_insert_with(|| {
                        self.starts.fetch_add(1, Ordering::SeqCst);
                        assignment.clone()
                    });
                if entry.request != assignment.request {
                    return RpcReply::error(409, "changed intent");
                }
                RpcReply::ok(json!(entry.index))
            }
            NodeOperation::Admission { command } => {
                if command == NodeAdmissionCommand::Freeze {
                    self.freezes.fetch_add(1, Ordering::SeqCst);
                }
                RpcReply::ok(json!({"mode":"open","active_runs":0,"owned_processes":0}))
            }
            NodeOperation::Brain { action, .. } if action == "intent" => {
                RpcReply::error(404, "unconfirmed intent")
            }
            NodeOperation::Brain { action, .. } if action == "capability_probe" => {
                RpcReply::ok(json!({"compatible":true,"features":["brain_scheduler_v4"]}))
            }
            NodeOperation::Events { after, .. } => {
                if after == 0 {
                    RpcReply::ok(
                        json!({"events":[{"seq":1,"kind":"log","data":{"text":"before"}}],"finished":false}),
                    )
                } else {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    RpcReply::ok(json!({"events":[],"finished":false}))
                }
            }
            _ => RpcReply::ok(json!({})),
        }
    }
}
struct Server {
    state: Arc<opencoder_control::AppState>,
    url: String,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}
impl Drop for Server {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
        self.state.lifecycle.retire();
    }
}
async fn start(root: &std::path::Path, node: Arc<Node>) -> Server {
    let state = opencoder_control::new_state(root.join("work"), root.join("control"), None)
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = opencoder_control::build_app(state.clone(), Some("release-test".into()), false);
    let http = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let remote = url.clone();
    let channel = tokio::spawn(async move {
        let _ = opencoder_node::fleet::run(&remote, "release-test", node).await;
    });
    until(async || {
        state
            .hub
            .views()
            .await
            .iter()
            .any(|n| n.online && n.snapshot.as_ref().is_some_and(|s| s.ready))
    })
    .await;
    Server {
        state,
        url,
        tasks: vec![http, channel],
    }
}
async fn until(mut predicate: impl AsyncFnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !predicate().await {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("handoff condition");
}
fn assignment() -> Assignment {
    Assignment {
        private_context: None,
        runtime: None,
        codex: None,
        definition: None,
        index: ExecutionIndex {
            id: "agent-handoff".into(),
            node_id: "node-release".into(),
            kind: ExecutionKind::Agent,
            created_at: 1,
            status: ExecutionStatus::Pending,
        },
        request: CreateExecution {
            id: "agent-handoff".into(),
            kind: ExecutionKind::Agent,
            target: Some("act".into()),
            node_id: Some("node-release".into()),
            input: json!({"prompt":"unchanged"}),
        },
    }
}
fn client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}

#[tokio::test]
async fn layered_retry_keeps_original_intent_after_server_handoff() {
    let root = tempfile::tempdir().unwrap();
    let _scope = opencoder_core::config::scoped_config_home(root.path().join("home"));
    let node = Node::new();
    let old = start(root.path(), node.clone()).await;
    let request = json!({"id":"brain-release","schema_version":4,"plan":{"schema_version":4,"title":"handoff","objective":"preserve frozen capabilities","nodes":[{"node_id":"work","title":"perform the task","capability_id":"builtin-agent-act"}],"edges":[]}});
    let post = |url: String, body: serde_json::Value| {
        client()
            .post(format!("{url}/api/brain/runs"))
            .bearer_auth("release-test")
            .json(&body)
            .send()
    };
    let first = post(old.url.clone(), request.clone()).await.unwrap();
    assert_eq!(first.status(), 202);
    let accepted: serde_json::Value = first.json().await.unwrap();
    old.state.fleet.put_definition("brain_capability","cap-new",&json!({"id":"cap-new","kind":"agent","target":"act","summary":"new release capability"})).await.unwrap();
    let new = start(root.path(), node.clone()).await;
    let repeated = post(new.url.clone(), request.clone()).await.unwrap();
    assert_eq!(repeated.status(), 202);
    assert_eq!(
        repeated.json::<serde_json::Value>().await.unwrap(),
        accepted
    );
    assert_eq!(node.starts.load(Ordering::SeqCst), 1);
    let mut different = request;
    different["plan"]["objective"] = json!("changed");
    let changed = post(new.url.clone(), different).await.unwrap();
    assert_eq!(changed.status(), 409);
}

#[tokio::test]
async fn prepared_dispatch_recovers_on_another_server_and_retries_execute_once() {
    let root = tempfile::tempdir().unwrap();
    let _scope = opencoder_core::config::scoped_config_home(root.path().join("home"));
    std::fs::create_dir(root.path().join("work")).unwrap();
    let node = Node::new();
    let a = assignment();
    let fingerprint = opencoder_core::token_hash(&serde_json::to_string(&a.request).unwrap());
    // Crash boundary: transaction committed, no RPC or acceptance response yet.
    let crashed =
        opencoder_control::new_state(root.path().join("work"), root.path().join("control"), None)
            .await
            .unwrap();
    assert!(crashed
        .fleet
        .claim_request("execution", &a.index.id, &fingerprint)
        .await
        .unwrap());
    crashed
        .fleet
        .prepare_assignment(&a, &fingerprint)
        .await
        .unwrap();
    drop(crashed);
    let old = start(root.path(), node.clone()).await;
    let new = start(root.path(), node.clone()).await;
    until(async || {
        new.state
            .fleet
            .receipt("execution", &a.index.id)
            .await
            .unwrap()
            .is_some_and(|r| r.phase == "accepted")
    })
    .await;
    let send = |url: String, request: CreateExecution| async move {
        client()
            .post(format!("{url}/api/executions"))
            .bearer_auth("release-test")
            .json(&request)
            .send()
            .await
            .unwrap()
    };
    let (one, two) = tokio::join!(
        send(old.url.clone(), a.request.clone()),
        send(new.url.clone(), a.request.clone())
    );
    assert_eq!(one.status(), 202);
    assert_eq!(two.status(), 202);
    assert_eq!(
        one.json::<serde_json::Value>().await.unwrap(),
        two.json::<serde_json::Value>().await.unwrap()
    );
    let mut changed = a.request;
    changed.input = json!({"prompt":"changed"});
    assert_eq!(send(new.url.clone(), changed).await.status(), 409);
    assert_eq!(node.starts.load(Ordering::SeqCst), 1);
    assert_eq!(node.freezes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn retirement_interrupts_a_slow_sse_poll_and_preserves_cursor_and_admission() {
    let root = tempfile::tempdir().unwrap();
    let _scope = opencoder_core::config::scoped_config_home(root.path().join("home"));
    std::fs::create_dir(root.path().join("work")).unwrap();
    let node = Node::new();
    let a = assignment();
    node.records
        .lock()
        .unwrap()
        .insert(a.index.id.clone(), a.clone());
    let old = start(root.path(), node.clone()).await;
    let mut stream = client()
        .get(format!("{}/api/executions/{}/events", old.url, a.index.id))
        .bearer_auth("release-test")
        .send()
        .await
        .unwrap();
    assert_eq!(stream.status(), 200);
    let first = String::from_utf8(stream.chunk().await.unwrap().unwrap().to_vec()).unwrap();
    assert!(first.contains("id: 1"), "{first}");
    tokio::time::sleep(Duration::from_millis(450)).await;
    let retired = client()
        .post(format!("{}/api/admin/release/retire", old.url))
        .bearer_auth("release-test")
        .send()
        .await
        .unwrap();
    assert_eq!(retired.status(), 200);
    let tail = tokio::time::timeout(Duration::from_secs(1), stream.text())
        .await
        .unwrap()
        .unwrap();
    assert!(
        tail.contains("event: reconnect") && tail.contains("id: 1"),
        "{tail}"
    );
    assert!(!tail.contains("stream_end"));
    assert!(old.state.admission.is_open().await);
    assert_eq!(node.freezes.load(Ordering::SeqCst), 0);
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn retiring_server_accepts_late_ingress_connections_until_workers_exit() {
    let root = tempfile::tempdir().unwrap();
    let _scope = opencoder_core::config::scoped_config_home(root.path().join("home"));
    let work = root.path().join("work");
    std::fs::create_dir(&work).unwrap();
    let node = Node::new();
    let successor = start(root.path(), node.clone()).await;
    let successor_port = reqwest::Url::parse(&successor.url).unwrap().port().unwrap();
    let reserved = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = reserved.local_addr().unwrap().port();
    drop(reserved);
    let server = tokio::spawn(opencoder_control::serve(
        "127.0.0.1".into(),
        port,
        false,
        work,
        Some(root.path().join("server")),
        "release-test".into(),
    ));
    let url = format!("http://127.0.0.1:{port}");
    until(async || {
        client()
            .get(format!("{url}/api/health"))
            .bearer_auth("release-test")
            .send()
            .await
            .is_ok_and(|response| response.status() == 200)
    })
    .await;
    let mut worker = tokio::process::Command::new("/bin/sleep")
        .arg("30")
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let pid = worker.id().unwrap();
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    let ticks: u64 = stat
        .rsplit_once(") ")
        .unwrap()
        .1
        .split_whitespace()
        .nth(19)
        .unwrap()
        .parse()
        .unwrap();
    for body in [
        json!({"unknown_frontier": []}),
        json!({"ingress_workers": [{"pid": pid, "start_ticks": ticks}]}),
        json!({"successor_port": port}),
    ] {
        let rejected = client()
            .post(format!("{url}/api/admin/release/retire"))
            .bearer_auth("release-test")
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(rejected.status(), 400);
        assert!(
            !server.is_finished(),
            "invalid retirement stopped the listener"
        );
    }
    let response = client()
        .post(format!("{url}/api/admin/release/retire"))
        .bearer_auth("release-test")
        .json(&json!({"ingress_workers":[{"pid":pid,"start_ticks":ticks}],"successor_port":successor_port}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let submitted = client()
        .post(format!("{url}/api/executions"))
        .bearer_auth("release-test")
        .json(&assignment().request)
        .send()
        .await
        .unwrap();
    assert_eq!(submitted.status(), 202);
    assert_eq!(node.starts.load(Ordering::SeqCst), 1);
    for _ in 0..5 {
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!server.is_finished(), "ingress still owns this listener");
        let late = client()
            .get(format!("{url}/api/health"))
            .bearer_auth("release-test")
            .send()
            .await
            .unwrap();
        assert_eq!(late.status(), 200);
    }
    worker.kill().await.unwrap();
    worker.wait().await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
