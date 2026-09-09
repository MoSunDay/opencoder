use super::*;
use futures::{channel::mpsc as futures_mpsc, StreamExt};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{watch, Semaphore};
use tokio_tungstenite::accept_async;

struct Service {
    records: Vec<ExecutionIndex>,
    indexes_sampled: AtomicBool,
}

#[async_trait::async_trait]
impl NodeService for Service {
    fn registration(&self) -> NodeRegistration {
        unreachable!()
    }

    fn snapshot(&self) -> NodeSnapshot {
        assert!(
            self.indexes_sampled.load(Ordering::SeqCst),
            "report load snapshot must be sampled after its indexes"
        );
        NodeSnapshot {
            pending_runs: 0,
            queue_order: Default::default(),
            generation: "generation-a".into(),
            sequence: 2,
            cpu_capacity: 2.0,
            active_agent_loops: 1,
            active_runs: 1,
            max_runs: 2,
            ready: true,
            resource_error: None,
        }
    }

    fn changes(&self) -> tokio::sync::watch::Receiver<u64> {
        unreachable!()
    }

    async fn indexes(&self) -> anyhow::Result<Vec<ExecutionIndex>> {
        self.indexes_sampled.store(true, Ordering::SeqCst);
        Ok(self.records.clone())
    }

    async fn handle(&self, operation: NodeOperation) -> RpcReply {
        match operation {
            NodeOperation::Create { assignment } => {
                RpcReply::ok(serde_json::json!(assignment.index))
            }
            _ => unreachable!(),
        }
    }
}

fn record(index: usize) -> ExecutionIndex {
    ExecutionIndex {
        id: format!("agent-{index}"),
        created_at: index as i64,
        kind: ExecutionKind::Agent,
        node_id: "node-a".into(),
        status: ExecutionStatus::Running,
    }
}

async fn frames(records: Vec<ExecutionIndex>) -> Vec<NodeFrame> {
    let service = Service {
        records,
        indexes_sampled: AtomicBool::new(false),
    };
    let (mut writer, reader) = futures_mpsc::unbounded();
    let report = prepare_report(&service).await.unwrap();
    publish_report(&mut writer, report, 7).await.unwrap();
    drop(writer);
    reader
        .map(|message| match message {
            Message::Text(text) => serde_json::from_str(&text).unwrap(),
            other => panic!("unexpected report message: {other:?}"),
        })
        .collect()
        .await
}

#[tokio::test]
async fn empty_report_orders_snapshot_begin_and_end() {
    let frames = frames(vec![]).await;
    assert!(matches!(frames[0], NodeFrame::Snapshot { .. }));
    assert!(matches!(
        frames[1],
        NodeFrame::IndexReport {
            report: IndexReportEnvelope {
                report_id: 7,
                part: IndexReportPart::Begin
            }
        }
    ));
    assert!(matches!(
        frames[2],
        NodeFrame::IndexReport {
            report: IndexReportEnvelope {
                report_id: 7,
                part: IndexReportPart::End
            }
        }
    ));
}

#[tokio::test]
async fn report_chunks_more_than_one_batch() {
    let frames = frames((0..257).map(record).collect()).await;
    let sizes: Vec<_> = frames
        .iter()
        .filter_map(|frame| match frame {
            NodeFrame::IndexReport {
                report:
                    IndexReportEnvelope {
                        part: IndexReportPart::Batch { records },
                        ..
                    },
            } => Some(records.len()),
            _ => None,
        })
        .collect();
    assert_eq!(sizes, [INDEX_REPORT_BATCH_SIZE, 1]);
    assert!(matches!(
        frames.last(),
        Some(NodeFrame::IndexReport {
            report: IndexReportEnvelope {
                part: IndexReportPart::End,
                ..
            }
        })
    ));
}

#[tokio::test]
async fn create_reply_is_queued_after_its_load_snapshot() {
    let service: Arc<dyn NodeService> = Arc::new(Service {
        records: vec![record(1)],
        indexes_sampled: AtomicBool::new(true),
    });
    let (tx, mut rx) = mpsc::channel(2);
    let (trigger, mut triggers) = mpsc::channel(1);
    execute_call(
        service,
        NodeOperation::Create {
            assignment: Assignment {
                codex: None,
                index: record(1),
                request: CreateExecution {
                    id: "agent-1".into(),
                    kind: ExecutionKind::Agent,
                    target: None,
                    input: serde_json::Value::Null,
                    node_id: Some("node-a".into()),
                },
                definition: None,
            },
        },
        "request-1".into(),
        tx,
        trigger,
    )
    .await;
    assert!(matches!(rx.recv().await, Some(NodeFrame::Snapshot { .. })));
    assert!(matches!(
        rx.recv().await,
        Some(NodeFrame::Reply { request_id, .. }) if request_id == "request-1"
    ));
    assert_eq!(triggers.recv().await, Some(()));
}

struct SlowIndexes {
    started: Arc<Semaphore>,
    cancelled: Arc<Semaphore>,
    changes: watch::Sender<u64>,
}

struct CollectionGuard(Arc<Semaphore>);

impl Drop for CollectionGuard {
    fn drop(&mut self) {
        self.0.add_permits(1);
    }
}

#[async_trait::async_trait]
impl NodeService for SlowIndexes {
    fn registration(&self) -> NodeRegistration {
        NodeRegistration {
            protocol_version: PROTOCOL_VERSION,
            id: "node-slow".into(),
            name: "slow test node".into(),
            version: "test".into(),
            maintenance_agent_id: "maintenance-slow".into(),
            kinds: vec![ExecutionKind::Agent],
        }
    }

    fn snapshot(&self) -> NodeSnapshot {
        NodeSnapshot {
            pending_runs: 0,
            queue_order: Default::default(),
            generation: "generation-slow".into(),
            sequence: 1,
            cpu_capacity: 1.0,
            active_agent_loops: 0,
            active_runs: 1,
            max_runs: 1,
            ready: true,
            resource_error: None,
        }
    }

    fn changes(&self) -> tokio::sync::watch::Receiver<u64> {
        self.changes.subscribe()
    }

    async fn indexes(&self) -> anyhow::Result<Vec<ExecutionIndex>> {
        self.started.add_permits(1);
        let _guard = CollectionGuard(self.cancelled.clone());
        std::future::pending().await
    }

    async fn handle(&self, operation: NodeOperation) -> RpcReply {
        assert!(matches!(operation, NodeOperation::Inspect { .. }));
        RpcReply::ok(serde_json::json!({"visible":true}))
    }
}

#[tokio::test]
async fn connection_reads_calls_while_indexes_wait_and_cancels_collection_on_close() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let (changes, _) = watch::channel(0);
    let service = Arc::new(SlowIndexes {
        started: Arc::new(Semaphore::new(0)),
        cancelled: Arc::new(Semaphore::new(0)),
        changes,
    });
    let server_service = service.clone();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        let hello = socket.next().await.unwrap().unwrap();
        assert!(matches!(
            serde_json::from_str::<NodeFrame>(hello.to_text().unwrap()).unwrap(),
            NodeFrame::Hello { .. }
        ));
        server_service.started.acquire().await.unwrap().forget();
        socket
            .send(Message::Text(
                serde_json::to_string(&ServerFrame::Call {
                    request_id: "inspect-while-indexing".into(),
                    operation: NodeOperation::Inspect {
                        execution: ExecutionRef {
                            id: "agent-visible".into(),
                            kind: ExecutionKind::Agent,
                        },
                    },
                })
                .unwrap(),
            ))
            .await
            .unwrap();
        let reply = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                let message = socket.next().await.unwrap().unwrap();
                let frame: NodeFrame = serde_json::from_str(message.to_text().unwrap()).unwrap();
                if let NodeFrame::Reply { request_id, reply } = frame {
                    break (request_id, reply);
                }
            }
        })
        .await
        .expect("Inspect reply must not wait for index collection");
        assert_eq!(reply.0, "inspect-while-indexing");
        assert_eq!(reply.1.status, 200);
        socket.close(None).await.unwrap();
    });
    let client_service: Arc<dyn NodeService> = service.clone();
    let client = tokio::spawn(async move {
        connection(
            &url,
            "test-token",
            client_service,
            Arc::new(Semaphore::new(8)),
        )
        .await
    });

    server.await.unwrap();
    assert!(client.await.unwrap().is_err());
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        service.cancelled.acquire(),
    )
    .await
    .expect("closing the connection must cancel its index collection")
    .unwrap()
    .forget();
}
