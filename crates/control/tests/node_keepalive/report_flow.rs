//! Full report backpressure stays below the transport's bounded input queue.
use opencoder_core::fleet::*;
use opencoder_node::fleet::NodeService;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::watch;

struct Inventory {
    changes: watch::Sender<u64>,
    reports: AtomicUsize,
    calls: AtomicUsize,
    sequence: AtomicUsize,
}

#[async_trait::async_trait]
impl NodeService for Inventory {
    fn supports_index_deltas(&self) -> bool {
        true
    }
    fn registration(&self) -> NodeRegistration {
        NodeRegistration {
            protocol_version: PROTOCOL_VERSION,
            id: "node-flow".into(),
            name: "flow".into(),
            version: "test".into(),
            maintenance_agent_id: "maintainer-flow".into(),
            kinds: vec![ExecutionKind::Dag],
        }
    }
    fn snapshot(&self) -> NodeSnapshot {
        NodeSnapshot {
            generation: "host-00000000000000000001-flow".into(),
            sequence: self.sequence.fetch_add(1, Ordering::SeqCst) as u64 + 1,
            cpu_capacity: 2.0,
            active_agent_loops: 0,
            active_runs: 1,
            pending_runs: 0,
            max_runs: 2,
            queue_order: Default::default(),
            ready: true,
            resource_error: None,
        }
    }
    fn changes(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }
    async fn indexes(&self) -> anyhow::Result<Vec<ExecutionIndex>> {
        let index = self.reports.fetch_add(1, Ordering::SeqCst);
        Ok((0..if index == 0 { 0 } else { 33000 })
            .map(|i| ExecutionIndex {
                id: format!("dag-flow-{i}"),
                created_at: i,
                kind: ExecutionKind::Dag,
                node_id: "node-flow".into(),
                status: ExecutionStatus::Done,
            })
            .collect())
    }
    async fn handle(&self, operation: NodeOperation) -> RpcReply {
        match operation {
            NodeOperation::Admission { .. } => RpcReply::ok(serde_json::json!({"mode":"open"})),
            NodeOperation::Command { command, .. } => {
                assert_eq!(command.action, "cancel");
                self.calls.fetch_add(1, Ordering::SeqCst);
                RpcReply::ok(serde_json::json!({"cancelled":true}))
            }
            other => panic!("unexpected service call {other:?}"),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn slow_full_report_commit_preserves_heartbeat_and_owned_cancel_reply() {
    let root = tempfile::tempdir().unwrap();
    let _home = opencoder_core::config::scoped_config_home(root.path().join("home"));
    let state = opencoder_control::new_state(
        super::isolated_work(root.path()),
        root.path().join("data"),
        None,
    )
    .await
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let remote = format!("http://{}", listener.local_addr().unwrap());
    let app = opencoder_control::build_app(state.clone(), Some("flow-test".into()), false);
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (changes, _) = watch::channel(0);
    let inventory = Arc::new(Inventory {
        changes,
        reports: AtomicUsize::new(0),
        calls: AtomicUsize::new(0),
        sequence: AtomicUsize::new(0),
    });
    let service: Arc<dyn NodeService> = inventory.clone();
    let node =
        tokio::spawn(
            async move { opencoder_node::fleet::run(&remote, "flow-test", service).await },
        );
    let result = tokio::time::timeout(Duration::from_secs(35), async {
        while !state
            .hub
            .views()
            .await
            .iter()
            .any(|v| v.snapshot.as_ref().is_some_and(|s| s.ready))
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let db = turso::Builder::new_local(
            (root.path().join("data/control.db"))
                .to_str()
                .expect("UTF-8 database path"),
        )
        .experimental_multiprocess_wal(true)
        .build()
        .await
        .unwrap();
        let writer = db.connect().unwrap();
        writer.execute("BEGIN IMMEDIATE", ()).await.unwrap();
        inventory.changes.send(1).unwrap();
        while inventory.reports.load(Ordering::SeqCst) < 2 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            tokio::time::timeout(
                Duration::from_millis(20),
                state
                    .fleet
                    .put_definition("test", "writer-barrier", &serde_json::json!({})),
            )
            .await
            .is_err()
        );
        assert!(
            !tokio::time::timeout(Duration::from_secs(1), state.fleet.nodes())
                .await
                .unwrap()
                .unwrap()
                .is_empty()
        );
        let caller = state.clone();
        let cancel = tokio::spawn(async move {
            caller
                .hub
                .call(
                    "node-flow",
                    NodeOperation::Command {
                        execution: ExecutionRef {
                            id: "dag-flow-0".into(),
                            kind: ExecutionKind::Dag,
                        },
                        command: ExecutionCommand {
                            action: "cancel".into(),
                            input: serde_json::Value::Null,
                        },
                    },
                )
                .await
        });
        while inventory.calls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        let before = state.hub.views().await.remove(0);
        // More than STALE_MS, while triggering inventories that would otherwise
        // fill 128 incoming slots with 130+ batches of the next complete report.
        for revision in 2..=23 {
            inventory.changes.send(revision).unwrap();
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        let after = state.hub.views().await.remove(0);
        let reports_before_commit = inventory.reports.load(Ordering::SeqCst);
        writer.execute("COMMIT", ()).await.unwrap();
        assert!(
            after.online,
            "a live peer disconnected under report storage backpressure"
        );
        assert!(
            after.last_seen_at > before.last_seen_at,
            "heartbeats stopped behind storage"
        );
        assert_eq!(
            reports_before_commit, 2,
            "unapplied reports must not pile up"
        );
        let reply = tokio::time::timeout(Duration::from_secs(5), cancel)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            reply.status, 200,
            "the reply must remain attached to its original call"
        );
        while state.fleet.index("dag-flow-32999").await.unwrap().is_none() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let row = state.fleet.index("dag-flow-32999").await.unwrap().unwrap();
        assert_eq!(row.node_id, "node-flow");
        assert_eq!(row.status, ExecutionStatus::Done);
    })
    .await;
    node.abort();
    server.abort();
    result.expect("report flow-control scenario exceeded its bounded budget");
}

async fn report_ack(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    id: u64,
    sequence: u64,
) {
    use futures::StreamExt;
    tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) = socket.next().await {
            let ServerFrame::Call { request_id, operation } = serde_json::from_str(&text).unwrap();
            if request_id == format!("report-ack-{id}-{sequence}") {
                assert!(matches!(operation, NodeOperation::Maintenance { command } if command.action == INDEX_REPORT_ACK_ACTION));
                return;
            }
        }
        panic!("report acknowledgement connection closed");
    }).await.expect("report acknowledgement timed out");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn acknowledged_report_detach_cannot_promote_or_ack_replacement_connection() {
    use super::report_storage::{Fixture, send, until};
    use futures::StreamExt;
    let fixture = Fixture::start().await;
    let mut old = fixture.connect(1).await;
    send(
        &mut old,
        NodeFrame::IndexReport {
            report: IndexReportEnvelope::begin(1).with_ack(true),
        },
    )
    .await;
    report_ack(&mut old, 1, 0).await;
    let writer = fixture.writer().await;
    send(
        &mut old,
        NodeFrame::IndexReport {
            report: IndexReportEnvelope::end(1).with_ack(true),
        },
    )
    .await;
    fixture.blocked().await;
    old.close(None).await.unwrap();
    until(async || !fixture.state.hub.views().await[0].online).await;
    let mut new = fixture.open(10).await;
    writer.execute("COMMIT", ()).await.unwrap();
    fixture
        .state
        .fleet
        .put_definition("test", "register-barrier", &serde_json::json!({}))
        .await
        .unwrap();
    let view = fixture.state.hub.views().await.remove(0);
    assert!(view.online);
    assert!(!view.snapshot.as_ref().unwrap().ready);
    assert_eq!(view.snapshot.unwrap().sequence, 10);
    assert!(
        tokio::time::timeout(Duration::from_millis(50), new.next())
            .await
            .is_err(),
        "closed report must not acknowledge or promote its replacement"
    );
    send(
        &mut new,
        NodeFrame::IndexReport {
            report: IndexReportEnvelope::begin(1).with_ack(true),
        },
    )
    .await;
    report_ack(&mut new, 1, 0).await;
    send(
        &mut new,
        NodeFrame::IndexReport {
            report: IndexReportEnvelope::end(1).with_ack(true),
        },
    )
    .await;
    report_ack(&mut new, 1, 1).await;
    assert!(
        fixture.state.hub.views().await[0]
            .snapshot
            .as_ref()
            .unwrap()
            .ready
    );
    new.close(None).await.unwrap();
}
