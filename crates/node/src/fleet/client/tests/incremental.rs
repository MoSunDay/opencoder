use super::*;
use std::sync::{atomic::AtomicU64, Mutex};

struct Changing {
    changes: watch::Sender<u64>,
    records: Mutex<Vec<ExecutionIndex>>,
    sequence: AtomicU64,
}
#[async_trait::async_trait]
impl NodeService for Changing {
    fn supports_index_deltas(&self) -> bool {
        true
    }
    fn registration(&self) -> NodeRegistration {
        NodeRegistration {
            protocol_version: PROTOCOL_VERSION,
            id: "node-a".into(),
            name: "incremental".into(),
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
        snapshot.generation = "host-00000000000000000001-test".into();
        snapshot.sequence = self.sequence.fetch_add(1, Ordering::SeqCst) + 1;
        snapshot
    }
    fn changes(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }
    async fn indexes(&self) -> Result<Vec<ExecutionIndex>> {
        Ok(self.records.lock().unwrap().clone())
    }
    async fn brain_frames(&self) -> Result<Vec<NodeFrame>> {
        Ok(vec![NodeFrame::Brain {
            execution: record(1).execution_ref(),
            action: "test-brain".into(),
            input: serde_json::Value::Null,
        }])
    }
    async fn handle(&self, _: NodeOperation) -> RpcReply {
        RpcReply::ok(serde_json::json!({"alive":true}))
    }
}
async fn next(socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>) -> NodeFrame {
    loop {
        match socket.next().await.unwrap().unwrap() {
            Message::Text(text) => return serde_json::from_str(&text).unwrap(),
            Message::Ping(data) => socket.send(Message::Pong(data)).await.unwrap(),
            other => panic!("unexpected {other:?}"),
        }
    }
}
async fn ack(
    socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    id: u64,
    sequence: u64,
) {
    socket
        .send(Message::Text(
            serde_json::to_string(&ServerFrame::Call {
                request_id: format!("report-ack-{id}-{sequence}"),
                operation: NodeOperation::Maintenance {
                    command: ExecutionCommand {
                        action: INDEX_REPORT_ACK_ACTION.into(),
                        input: serde_json::json!({"report_id":id,"sequence":sequence}),
                    },
                },
            })
            .unwrap(),
        ))
        .await
        .unwrap();
}

struct IncrementalHandshake;
impl tokio_tungstenite::tungstenite::handshake::server::Callback for IncrementalHandshake {
    fn on_request(
        self,
        request: &tokio_tungstenite::tungstenite::handshake::server::Request,
        mut response: tokio_tungstenite::tungstenite::handshake::server::Response,
    ) -> std::result::Result<
        tokio_tungstenite::tungstenite::handshake::server::Response,
        tokio_tungstenite::tungstenite::handshake::server::ErrorResponse,
    > {
        assert_eq!(
            request.headers()[INDEX_REPORT_DELTA_HEADER],
            INDEX_REPORT_DELTA_VERSION
        );
        assert_eq!(request.headers()[INDEX_REPORT_ACK_HEADER], "1");
        response
            .headers_mut()
            .insert(INDEX_REPORT_ACK_HEADER, "1".parse().unwrap());
        response.headers_mut().insert(
            INDEX_REPORT_DELTA_HEADER,
            INDEX_REPORT_DELTA_VERSION.parse().unwrap(),
        );
        Ok(response)
    }
}

#[tokio::test]
async fn delta_waits_for_commit_while_brain_and_rpc_continue_and_idle_has_no_report() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let service = Arc::new(Changing {
        changes: watch::channel(0).0,
        records: Mutex::new(vec![record(1), record(2)]),
        sequence: AtomicU64::new(0),
    });
    let source = service.clone();
    let client = tokio::spawn(async move {
        connection_with_period(
            &url,
            "test-token",
            source,
            Arc::new(Semaphore::new(128)),
            std::time::Duration::from_millis(20),
        )
        .await
    });
    let result = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_hdr_async(stream, IncrementalHandshake)
            .await
            .unwrap();
        let mut full_sequence = 0;
        let mut fragment = 0;
        loop {
            match next(&mut socket).await {
                NodeFrame::Snapshot { snapshot } => full_sequence = snapshot.sequence,
                NodeFrame::IndexReport { report } => {
                    assert_eq!(report.report_id, 1);
                    if matches!(report.part, IndexReportPart::End) {
                        break;
                    }
                    assert_eq!(report.mode, IndexReportMode::Full);
                    ack(&mut socket, 1, fragment).await;
                    fragment += 1;
                }
                _ => {}
            }
        }
        // Mutations while End waits for storage are coalesced against the committed full image.
        {
            let mut records = service.records.lock().unwrap();
            records[0].status = ExecutionStatus::Done;
            records.pop();
        }
        service.changes.send(1).unwrap();
        socket
            .send(Message::Text(
                serde_json::to_string(&ServerFrame::Call {
                    request_id: "rpc-during-commit".into(),
                    operation: NodeOperation::Inspect {
                        execution: record(1).execution_ref(),
                    },
                })
                .unwrap(),
            ))
            .await
            .unwrap();
        let mut brains = 0;
        let mut replied = false;
        while brains < 2 || !replied {
            match next(&mut socket).await {
                NodeFrame::Brain { .. } => brains += 1,
                NodeFrame::Reply { request_id, reply } => {
                    assert_eq!(request_id, "rpc-during-commit");
                    assert_eq!(reply.status, 200);
                    replied = true;
                }
                NodeFrame::IndexReport { .. } => panic!("new report before commit acknowledgement"),
                _ => {}
            }
        }
        ack(&mut socket, 1, fragment).await;
        let mut fragment = 0;
        let mut updated = false;
        loop {
            if let NodeFrame::IndexReport { report } = next(&mut socket).await {
                assert_eq!(report.report_id, 2);
                let end = matches!(report.part, IndexReportPart::End);
                match report.part {
                    IndexReportPart::Begin => {
                        assert_eq!(report.mode, IndexReportMode::Delta);
                        assert_eq!(report.base_sequence, Some(full_sequence));
                    }
                    IndexReportPart::Batch { records } => {
                        assert_eq!(records.len(), 1);
                        assert_eq!(records[0].status, ExecutionStatus::Done);
                        assert_eq!(report.removed_ids, ["agent-2"]);
                        updated = true;
                    }
                    IndexReportPart::End => assert!(updated),
                }
                ack(&mut socket, 2, fragment).await;
                fragment += 1;
                if end {
                    break;
                }
            }
        }
        let mut snapshots = 0;
        while snapshots < 3 {
            match next(&mut socket).await {
                NodeFrame::Snapshot { .. } => snapshots += 1,
                NodeFrame::IndexReport { .. } => panic!("unchanged inventory sent another report"),
                _ => {}
            }
        }
        socket.close(None).await.unwrap();
    })
    .await;
    client.abort();
    result.unwrap();
}

#[tokio::test]
async fn legacy_server_receives_acknowledged_full_reports_after_changes() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let service = Arc::new(Changing {
        changes: watch::channel(0).0,
        records: Mutex::new(vec![record(1), record(2)]),
        sequence: AtomicU64::new(0),
    });
    let source = service.clone();
    let client = tokio::spawn(async move {
        connection_with_period(
            &url,
            "test-token",
            source,
            Arc::new(Semaphore::new(128)),
            std::time::Duration::from_millis(20),
        )
        .await
    });
    let result = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_hdr_async(stream, LegacyHandshake)
            .await
            .unwrap();
        for report_id in 1..=2 {
            let mut fragment = 0;
            let mut records = Vec::new();
            loop {
                if let NodeFrame::IndexReport { report } = next(&mut socket).await {
                    assert_eq!(report.report_id, report_id);
                    assert!(report.acknowledge);
                    assert_eq!(report.mode, IndexReportMode::Full);
                    assert_eq!(report.base_sequence, None);
                    assert!(report.removed_ids.is_empty());
                    let end = matches!(report.part, IndexReportPart::End);
                    if let IndexReportPart::Batch { records: batch } = report.part {
                        records.extend(batch);
                    }
                    if end && report_id == 1 {
                        service.records.lock().unwrap()[0].status = ExecutionStatus::Done;
                        service.changes.send(1).unwrap();
                    }
                    ack(&mut socket, report_id, fragment).await;
                    fragment += 1;
                    if end {
                        break;
                    }
                }
            }
            assert_eq!(
                records.len(),
                2,
                "legacy peers must receive unchanged rows too"
            );
            if report_id == 2 {
                assert_eq!(records[0].status, ExecutionStatus::Done);
            }
        }
        socket.close(None).await.unwrap();
    })
    .await;
    client.abort();
    result.unwrap();
}

struct LegacyHandshake;
impl tokio_tungstenite::tungstenite::handshake::server::Callback for LegacyHandshake {
    fn on_request(
        self,
        request: &tokio_tungstenite::tungstenite::handshake::server::Request,
        mut response: tokio_tungstenite::tungstenite::handshake::server::Response,
    ) -> std::result::Result<
        tokio_tungstenite::tungstenite::handshake::server::Response,
        tokio_tungstenite::tungstenite::handshake::server::ErrorResponse,
    > {
        // Match the old Server's actual version-1 negotiation, not a permissive echo.
        assert_eq!(request.headers()[INDEX_REPORT_DELTA_HEADER], "2");
        response
            .headers_mut()
            .insert(INDEX_REPORT_ACK_HEADER, "1".parse().unwrap());
        if request
            .headers()
            .get(INDEX_REPORT_DELTA_HEADER)
            .is_some_and(|v| v == "1")
        {
            response
                .headers_mut()
                .insert(INDEX_REPORT_DELTA_HEADER, "1".parse().unwrap());
        }
        Ok(response)
    }
}
