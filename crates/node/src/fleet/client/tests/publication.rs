use super::*;

struct LargeReport {
    changes: watch::Sender<u64>,
    reported: AtomicBool,
}

#[async_trait::async_trait]
impl NodeService for LargeReport {
    fn registration(&self) -> NodeRegistration {
        NodeRegistration {
            protocol_version: PROTOCOL_VERSION,
            id: "node-large".into(),
            name: "large report".into(),
            version: "test".into(),
            maintenance_agent_id: "maintenance-large".into(),
            kinds: vec![ExecutionKind::Agent],
        }
    }
    fn snapshot(&self) -> NodeSnapshot {
        Service {
            records: vec![],
            indexes_sampled: AtomicBool::new(true),
        }
        .snapshot()
    }
    fn changes(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }
    async fn indexes(&self) -> Result<Vec<ExecutionIndex>> {
        if self.reported.swap(true, Ordering::SeqCst) {
            std::future::pending().await
        } else {
            Ok((0..257).map(record).collect())
        }
    }
    async fn brain_frames(&self) -> Result<Vec<NodeFrame>> {
        Ok((0..180)
            .map(|i| NodeFrame::Brain {
                execution: record(i).execution_ref(),
                action: "test-payload".into(),
                input: serde_json::json!({"padding":"x".repeat(262144)}),
            })
            .collect())
    }
    async fn handle(&self, _: NodeOperation) -> RpcReply {
        RpcReply::ok(serde_json::json!({"visible":true}))
    }
}

#[tokio::test]
async fn backpressured_publication_services_rpc_and_heartbeat_before_report_end() {
    publication(false).await;
}

#[tokio::test]
async fn acknowledged_publication_preserves_brain_order_rpc_and_heartbeat() {
    publication(true).await;
}

// Tungstenite requires its unboxed ErrorResponse in the handshake callback.
#[allow(clippy::result_large_err)]
async fn publication(acknowledged: bool) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let (changes, _) = watch::channel(0);
    let client = tokio::spawn(async move {
        connection_with_period(
            &url,
            "test-token",
            Arc::new(LargeReport {
                changes,
                reported: AtomicBool::new(false),
            }),
            Arc::new(Semaphore::new(128)),
            std::time::Duration::from_millis(50),
        )
        .await
    });
    let result = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_hdr_async(stream, move |_: &tokio_tungstenite::tungstenite::handshake::server::Request, mut response: tokio_tungstenite::tungstenite::handshake::server::Response| {
            if acknowledged { response.headers_mut().insert(INDEX_REPORT_ACK_HEADER, "1".parse().unwrap()); }
            Ok(response)
        }).await.unwrap();
        let mut sent_call = false;
        let mut reply_seen = false;
        let mut ping_seen = false;
        let mut batches = Vec::new();
        let mut brains = 0;
        let mut sequence = 0;
        loop {
            let message = socket.next().await.unwrap().unwrap();
            if let Message::Ping(data) = message {
                ping_seen = true;
                socket.send(Message::Pong(data)).await.unwrap();
                continue;
            }
            let frame: NodeFrame = serde_json::from_str(message.to_text().unwrap()).unwrap();
            let requires_ack = matches!(frame, NodeFrame::IndexReport { .. } | NodeFrame::Brain { .. });
            let is_begin = matches!(&frame, NodeFrame::IndexReport { report } if matches!(report.part, IndexReportPart::Begin));
            if requires_ack && !is_begin { sequence += 1; }
            if acknowledged && requires_ack {
                socket.send(Message::Text(serde_json::to_string(&ServerFrame::Call {
                    request_id: format!("report-ack-1-{sequence}"),
                    operation: NodeOperation::Maintenance { command: ExecutionCommand {
                        action: INDEX_REPORT_ACK_ACTION.into(), input: serde_json::json!({"report_id":1,"sequence":sequence})
                    }}
                }).unwrap())).await.unwrap();
            }
            match frame {
                NodeFrame::IndexReport { report } => match report.part {
                    IndexReportPart::Begin => {
                        assert_eq!(report.report_id, 1);
                        assert!(!sent_call, "only one in-flight report");
                        sent_call = true;
                        socket
                            .send(Message::Text(
                                serde_json::to_string(&ServerFrame::Call {
                                    request_id: "during-publication".into(),
                                    operation: NodeOperation::Inspect {
                                        execution: record(0).execution_ref(),
                                    },
                                })
                                .unwrap(),
                            ))
                            .await
                            .unwrap();
                    }
                    IndexReportPart::Batch { records } => {
                        batches.extend(records.into_iter().map(|r| r.id));
                    }
                    IndexReportPart::End => {
                        assert!(reply_seen, "RPC must finish while the report is being sent");
                        assert!(ping_seen, "report publication must not suppress heartbeats");
                        assert_eq!(batches, (0..257).map(|i| record(i).id).collect::<Vec<_>>());
                        assert_eq!(brains, 180);
                        break;
                    }
                },
                NodeFrame::Reply { request_id, reply } => {
                    assert_eq!(request_id, "during-publication");
                    assert_eq!(reply.status, 200);
                    reply_seen = true;
                }
                NodeFrame::Brain { .. } => {
                    brains += 1;
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
                NodeFrame::Hello { .. } | NodeFrame::Snapshot { .. } => {}
            }
        }
        socket.close(None).await.unwrap();
        while let Some(message) = socket.next().await {
            if matches!(message, Ok(Message::Close(_)) | Err(_)) {
                break;
            }
        }
    })
    .await;
    if result.is_err() {
        client.abort();
    }
    result.expect("bounded publication must keep its control channel live");
    tokio::time::timeout(std::time::Duration::from_secs(1), client)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
