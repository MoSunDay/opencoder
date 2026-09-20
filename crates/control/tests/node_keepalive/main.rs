//! Real Server/Node channel stays live without completing an inventory report.
use opencoder_core::fleet::*;
use opencoder_node::fleet::NodeService;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::sync::{watch, Semaphore};

struct SlowInventory {
    changes: watch::Sender<u64>,
    collecting: Semaphore,
    snapshots: AtomicUsize,
}

#[async_trait::async_trait]
impl NodeService for SlowInventory {
    fn registration(&self) -> NodeRegistration {
        NodeRegistration {
            protocol_version: PROTOCOL_VERSION,
            id: "node-keepalive".into(),
            name: "keepalive".into(),
            version: "test".into(),
            maintenance_agent_id: "act".into(),
            kinds: vec![ExecutionKind::Dag],
        }
    }
    fn snapshot(&self) -> NodeSnapshot {
        self.snapshots.fetch_add(1, Ordering::SeqCst);
        NodeSnapshot {
            pending_runs: 1,
            queue_order: Default::default(),
            generation: "keepalive-generation".into(),
            sequence: 1,
            cpu_capacity: 2.0,
            active_agent_loops: 0,
            active_runs: 0,
            max_runs: 2,
            ready: true,
            resource_error: None,
        }
    }
    fn changes(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }
    async fn indexes(&self) -> anyhow::Result<Vec<ExecutionIndex>> {
        self.collecting.add_permits(1);
        std::future::pending().await
    }
    async fn handle(&self, operation: NodeOperation) -> RpcReply {
        assert!(matches!(operation, NodeOperation::Admission { .. }));
        RpcReply::ok(serde_json::json!({"mode":"open"}))
    }
}

#[tokio::test]
async fn transport_ping_keeps_node_live_without_fabricating_load_or_inventory() {
    let directory = tempfile::tempdir().unwrap();
    let state = opencoder_control::new_state(
        directory.path().join("work"),
        directory.path().join("data"),
        None,
    )
    .await
    .unwrap();
    let app = opencoder_control::build_app(state.clone(), Some("keepalive-test".into()), false);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let remote = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let (changes, _) = watch::channel(0);
    let service = Arc::new(SlowInventory {
        changes,
        collecting: Semaphore::new(0),
        snapshots: AtomicUsize::new(0),
    });
    let node_service: Arc<dyn NodeService> = service.clone();
    let node = tokio::spawn(async move {
        opencoder_node::fleet::run(&remote, "keepalive-test", node_service).await
    });
    let result = tokio::time::timeout(std::time::Duration::from_secs(40), async {
        service.collecting.acquire().await.unwrap().forget();
        // Wait for Hello and the initial admission snapshot; deliberately
        // never finish the first report. No timing guess establishes readiness.
        while service.snapshots.load(Ordering::SeqCst) < 2 {
            tokio::task::yield_now().await;
        }
        let before = state.hub.views().await.remove(0);
        let snapshots = service.snapshots.load(Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(
            STALE_MS as u64 + HEARTBEAT_MS,
        ))
        .await;
        let after = state.hub.views().await.remove(0);
        assert!(
            after.online,
            "a live transport was declared offline during slow inventory"
        );
        assert!(after.last_seen_at > before.last_seen_at);
        assert_eq!(after.snapshot.unwrap().pending_runs, 1);
        assert_eq!(
            service.snapshots.load(Ordering::SeqCst),
            snapshots,
            "heartbeats must not invent fresher load snapshots"
        );
        assert!(!node.is_finished());
    })
    .await;
    node.abort();
    server.abort();
    result.expect("real keepalive scenario exceeded its bounded budget");
}
