use super::*;
use std::sync::atomic::AtomicUsize;

struct FailingInventory {
    base: SlowIndexes,
    recovered: AtomicBool,
    attempts: AtomicUsize,
}

#[async_trait::async_trait]
impl NodeService for FailingInventory {
    fn registration(&self) -> NodeRegistration {
        self.base.registration()
    }
    fn snapshot(&self) -> NodeSnapshot {
        self.base.snapshot()
    }
    fn changes(&self) -> watch::Receiver<u64> {
        self.base.changes()
    }
    async fn indexes(&self) -> Result<Vec<ExecutionIndex>> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        if !self.recovered.load(Ordering::SeqCst) {
            anyhow::bail!("old Runtime inventory timed out");
        }
        Ok(vec![ExecutionIndex {
            id: "agent-existing".into(),
            node_id: "node-slow".into(),
            kind: ExecutionKind::Agent,
            status: ExecutionStatus::Running,
            created_at: 10,
        }])
    }
    async fn handle(&self, operation: NodeOperation) -> RpcReply {
        self.base.handle(operation).await
    }
}

async fn frame(
    socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
) -> NodeFrame {
    let message = socket
        .next()
        .await
        .expect("inventory failure must not close the control channel")
        .expect("valid connection");
    serde_json::from_str(message.to_text().unwrap()).unwrap()
}

fn unavailable(frame: NodeFrame) {
    let NodeFrame::Snapshot { snapshot } = frame else {
        panic!("failed collection must report unavailable without an index report");
    };
    assert!(!snapshot.ready, "failed collection cannot claim readiness");
    assert_eq!(
        snapshot.resource_error.as_deref(),
        Some("old Runtime inventory timed out")
    );
    assert_eq!(snapshot.active_runs, 1, "failure must not clear known load");
}

#[tokio::test]
async fn inventory_failure_keeps_calls_and_heartbeats_then_retries_a_complete_report() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let (changes, _) = watch::channel(0);
    let service = Arc::new(FailingInventory {
        base: SlowIndexes {
            started: Arc::new(Semaphore::new(0)),
            cancelled: Arc::new(Semaphore::new(0)),
            changes,
        },
        recovered: AtomicBool::new(false),
        attempts: AtomicUsize::new(0),
    });
    let client_service: Arc<dyn NodeService> = service.clone();
    let client = tokio::spawn(async move {
        connection(
            &url,
            "test-token",
            client_service,
            Arc::new(Semaphore::new(128)),
        )
        .await
    });
    let result = tokio::time::timeout(std::time::Duration::from_secs(8), async {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        assert!(matches!(frame(&mut socket).await, NodeFrame::Hello { .. }));
        unavailable(frame(&mut socket).await);
        service.base.changes.send(1).unwrap();
        unavailable(frame(&mut socket).await);
        socket
            .send(Message::Text(
                serde_json::to_string(&ServerFrame::Call {
                    request_id: "inspect-after-failure".into(),
                    operation: NodeOperation::Inspect {
                        execution: ExecutionRef {
                            id: "agent-existing".into(),
                            kind: ExecutionKind::Agent,
                        },
                    },
                })
                .unwrap(),
            ))
            .await
            .unwrap();
        let NodeFrame::Reply { request_id, reply } = frame(&mut socket).await else {
            panic!("inspection must remain usable while inventory is unavailable");
        };
        assert_eq!(request_id, "inspect-after-failure");
        assert_eq!(reply.status, 200);
        let ping = socket.next().await.unwrap().unwrap();
        let Message::Ping(payload) = ping else {
            panic!("expected heartbeat after collection errors, got {ping:?}");
        };
        socket.send(Message::Pong(payload)).await.unwrap();
        service.recovered.store(true, Ordering::SeqCst);
        service.base.changes.send(2).unwrap();
        let mut records = Vec::new();
        loop {
            match frame(&mut socket).await {
                NodeFrame::Snapshot { snapshot } if snapshot.ready => {}
                NodeFrame::Snapshot { snapshot } => {
                    assert_eq!(
                        snapshot.resource_error.as_deref(),
                        Some("old Runtime inventory timed out")
                    );
                }
                NodeFrame::IndexReport { report } => {
                    assert_eq!(
                        report.report_id, 1,
                        "failed collections do not consume report IDs"
                    );
                    match report.part {
                        IndexReportPart::Begin => {}
                        IndexReportPart::Batch { records: batch } => records.extend(batch),
                        IndexReportPart::End => break,
                    }
                }
                other => panic!("unexpected recovery frame: {other:?}"),
            }
        }
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, "agent-existing");
        assert_eq!(records[0].status, ExecutionStatus::Running);
        assert!(service.attempts.load(Ordering::SeqCst) >= 3);
        socket.close(None).await.unwrap();
    })
    .await;
    if result.is_err() {
        client.abort();
    }
    result.expect("inventory recovery must preserve the original connection");
    client.await.unwrap().unwrap();
}
