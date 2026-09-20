use super::*;

struct RetiringAdmission(Arc<AdmissionDuringReport>);

#[async_trait::async_trait]
impl NodeService for RetiringAdmission {
    fn registration(&self) -> NodeRegistration {
        self.0.registration()
    }
    fn snapshot(&self) -> NodeSnapshot {
        self.0.snapshot()
    }
    fn changes(&self) -> watch::Receiver<u64> {
        self.0.changes()
    }
    fn retiring(&self) -> bool {
        true
    }
    async fn indexes(&self) -> Result<Vec<ExecutionIndex>> {
        self.0.indexes().await
    }
    async fn handle(&self, operation: NodeOperation) -> RpcReply {
        self.0.handle(operation).await
    }
}

#[tokio::test]
async fn connection_heartbeats_while_admission_and_inventory_are_blocked() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let (changes, _) = watch::channel(0);
    let service = Arc::new(AdmissionDuringReport {
        base: SlowIndexes {
            started: Arc::new(Semaphore::new(0)),
            cancelled: Arc::new(Semaphore::new(0)),
            changes,
        },
        gate: tokio::sync::Mutex::new(()),
        admission_started: Semaphore::new(0),
        release_report: Semaphore::new(0),
    });
    let client_service: Arc<dyn NodeService> = Arc::new(RetiringAdmission(service.clone()));
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
        socket.next().await.unwrap().unwrap();
        service.base.started.acquire().await.unwrap().forget();
        socket
            .send(Message::Text(
                serde_json::to_string(&ServerFrame::Call {
                    request_id: "blocked-admission".into(),
                    operation: NodeOperation::Admission {
                        command: NodeAdmissionCommand::Status,
                    },
                })
                .unwrap(),
            ))
            .await
            .unwrap();
        service.admission_started.acquire().await.unwrap().forget();
        let ping = socket.next().await.unwrap().unwrap();
        let Message::Ping(payload) = ping else {
            panic!("expected independent heartbeat, got {ping:?}");
        };
        socket.send(Message::Pong(payload)).await.unwrap();
        socket.close(None).await.unwrap();
    })
    .await;
    if result.is_err() {
        client.abort();
    }
    result.expect("blocked admission/index collection must not suppress transport heartbeat");
    tokio::time::timeout(std::time::Duration::from_secs(1), client)
        .await
        .expect("peer close must cancel the blocked admission worker")
        .unwrap()
        .unwrap();
}
