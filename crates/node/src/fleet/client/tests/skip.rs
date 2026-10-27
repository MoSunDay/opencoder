use super::*;
use std::sync::Mutex;

/// A legacy full reporter: identical inventory every collection until the
/// test flips `generation`, then one record changes.
struct Steady {
    changes: watch::Sender<u64>,
    records: Mutex<Vec<ExecutionIndex>>,
    flip: AtomicBool,
}

#[async_trait::async_trait]
impl NodeService for Steady {
    fn registration(&self) -> NodeRegistration {
        NodeRegistration {
            protocol_version: PROTOCOL_VERSION,
            id: "node-steady".into(),
            name: "steady".into(),
            version: "test".into(),
            maintenance_agent_id: "maintenance".into(),
            kinds: vec![ExecutionKind::Agent],
        }
    }
    fn snapshot(&self) -> NodeSnapshot {
        let mut snapshot = Service {
            records: vec![],
            indexes_sampled: AtomicBool::new(true),
        }
        .snapshot();
        snapshot.ready = !self.flip.load(Ordering::SeqCst);
        snapshot
    }
    fn changes(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }
    async fn indexes(&self) -> Result<Vec<ExecutionIndex>> {
        Ok(self.records.lock().unwrap().clone())
    }
    async fn handle(&self, _: NodeOperation) -> RpcReply {
        RpcReply::ok(serde_json::json!({"alive":true}))
    }
}

async fn frame(
    socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
) -> NodeFrame {
    loop {
        match socket.next().await.unwrap().unwrap() {
            Message::Text(text) => return serde_json::from_str(&text).unwrap(),
            Message::Ping(data) => socket.send(Message::Pong(data)).await.unwrap(),
            other => panic!("unexpected {other:?}"),
        }
    }
}

#[tokio::test]
async fn unchanged_full_inventory_is_not_reuploaded_until_it_changes() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let (changes, _) = watch::channel(0);
    let service = Arc::new(Steady {
        changes,
        records: Mutex::new((0..300).map(record).collect()),
        flip: AtomicBool::new(false),
    });
    let client_service: Arc<dyn NodeService> = service.clone();
    let period = std::time::Duration::from_millis(50);
    let client = tokio::spawn(async move {
        connection_with_period(&url, "test-token", client_service, Arc::new(Semaphore::new(128)), period).await
    });
    let result = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        // Hello, then the initial full report: begin + batches + end.
        assert!(matches!(frame(&mut socket).await, NodeFrame::Hello { .. }));
        // A full report opens with a load snapshot, then the begin envelope.
        assert!(matches!(frame(&mut socket).await, NodeFrame::Snapshot { .. }));
        assert!(matches!(
            frame(&mut socket).await,
            NodeFrame::IndexReport { report: IndexReportEnvelope { part: IndexReportPart::Begin, .. } }
        ));
        let mut batches = 0;
        loop {
            match frame(&mut socket).await {
                NodeFrame::IndexReport { report: IndexReportEnvelope { part: IndexReportPart::Batch { .. }, .. } } => batches += 1,
                NodeFrame::IndexReport { report: IndexReportEnvelope { part: IndexReportPart::End, .. } } => break,
                other => panic!("unexpected {other:?}"),
            }
        }
        assert_eq!(batches, 2);
        // Unchanged heartbeats publish load snapshots only.
        for _ in 0..3 {
            match frame(&mut socket).await {
                NodeFrame::Snapshot { snapshot } => assert!(snapshot.ready),
                other => panic!("expected snapshot, got {other:?}"),
            }
        }
        // A record change restores full publication.
        service.records.lock().unwrap().push(record(301));
        for _ in 0..8 {
            if let NodeFrame::IndexReport { report: IndexReportEnvelope { part: IndexReportPart::Begin, report_id: 2, .. } } =
                frame(&mut socket).await
            {
                return;
            }
        }
        panic!("changed inventory was not republished");
    })
    .await;
    client.abort();
    result.unwrap();
}
