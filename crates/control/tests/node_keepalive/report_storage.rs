//! Real transport during blocked, failed and superseded index persistence.
use futures::{SinkExt, StreamExt};
use opencoder_core::fleet::*;
use std::{sync::Arc, time::Duration};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
    MaybeTlsStream, WebSocketStream,
};

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
const GENERATION: &str = "host-00000000000000000001-storage";

pub(super) struct Fixture {
    root: tempfile::TempDir,
    pub(super) state: Arc<opencoder_control::AppState>,
    remote: String,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
impl Fixture {
    pub(super) async fn start() -> Self {
        let root = tempfile::tempdir().unwrap();
        let _scope = opencoder_core::config::scoped_config_home(root.path().join("config"));
        let state =
            opencoder_control::new_state(root.path().join("work"), root.path().join("data"), None)
                .await
                .unwrap();
        let app = opencoder_control::build_app(state.clone(), Some("storage-test".into()), false);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let remote = format!(
            "ws://{}/api/nodes/channel?node_id=node-storage",
            listener.local_addr().unwrap()
        );
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            root,
            state,
            remote,
            server,
        }
    }
    pub(super) async fn open(&self, sequence: u64) -> Socket {
        let mut request = self.remote.clone().into_client_request().unwrap();
        request
            .headers_mut()
            .insert("authorization", "Bearer storage-test".parse().unwrap());
        let (mut socket, _) = connect_async(request).await.unwrap();
        send(
            &mut socket,
            NodeFrame::Hello {
                registration: NodeRegistration {
                    id: "node-storage".into(),
                    name: "storage".into(),
                    version: "test".into(),
                    protocol_version: PROTOCOL_VERSION,
                    maintenance_agent_id: "maintenance-storage".into(),
                    kinds: vec![ExecutionKind::Dag],
                },
                snapshot: snapshot(sequence),
            },
        )
        .await;
        let admission = tokio::time::timeout(Duration::from_secs(3), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(matches!(admission, Message::Text(_)));
        socket
    }
    pub(super) async fn connect(&self, sequence: u64) -> Socket {
        let socket = self.open(sequence).await;
        until(async || !self.state.fleet.nodes().await.unwrap().is_empty()).await;
        socket
    }
    pub(super) async fn writer(&self) -> turso::Connection {
        let db = turso::Builder::new_local(
            (self.root.path().join("data/control.db"))
                .to_str()
                .expect("UTF-8 database path"),
        )
        .experimental_multiprocess_wal(true)
        .build()
        .await
        .unwrap();
        let writer = db.connect().unwrap();
        writer.execute("BEGIN IMMEDIATE", ()).await.unwrap();
        writer
    }
    pub(super) async fn blocked(&self) {
        let body = serde_json::json!({"waiting": true});
        assert!(
            tokio::time::timeout(
                Duration::from_millis(20),
                self.state
                    .fleet
                    .put_definition("test", "writer-barrier", &body),
            )
            .await
            .is_err(),
            "external transaction must block the writer"
        );
        // A read is no longer a writer barrier: the pool keeps WAL readers
        // available while a report waits for an external write transaction.
        assert!(
            !tokio::time::timeout(Duration::from_secs(1), self.state.fleet.nodes())
                .await
                .unwrap()
                .unwrap()
                .is_empty()
        );
    }
}
fn snapshot(sequence: u64) -> NodeSnapshot {
    NodeSnapshot {
        generation: GENERATION.into(),
        sequence,
        cpu_capacity: 2.0,
        active_agent_loops: 0,
        active_runs: 0,
        pending_runs: 1,
        max_runs: 2,
        queue_order: Default::default(),
        ready: true,
        resource_error: None,
    }
}
fn index(status: ExecutionStatus) -> ExecutionIndex {
    ExecutionIndex {
        id: "dag-storage".into(),
        node_id: "node-storage".into(),
        kind: ExecutionKind::Dag,
        created_at: 1,
        status,
    }
}
pub(super) async fn send(socket: &mut Socket, frame: NodeFrame) {
    socket
        .send(Message::Text(serde_json::to_string(&frame).unwrap()))
        .await
        .unwrap();
}
async fn report(socket: &mut Socket, report_id: u64, records: Vec<ExecutionIndex>) {
    for part in [
        IndexReportPart::Begin,
        IndexReportPart::Batch { records },
        IndexReportPart::End,
    ] {
        send(
            socket,
            NodeFrame::IndexReport {
                report: IndexReportEnvelope {
                    report_id,
                    part,
                    acknowledge: false,
                },
            },
        )
        .await;
    }
}
async fn handoff(socket: &mut Socket) -> bool {
    tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(Ok(message)) = socket.next().await {
            if let Message::Text(text) = message {
                let ServerFrame::Call { operation, .. } = serde_json::from_str(&text).unwrap();
                if let NodeOperation::Maintenance { command } = operation {
                    assert_eq!(command.action, "host_handoff_ready");
                    return true;
                }
            }
        }
        false
    })
    .await
    .expect("handoff acknowledgement timed out")
}
pub(super) async fn until(mut check: impl AsyncFnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !check().await {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("storage condition did not settle");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn locked_report_storage_keeps_ping_live_without_marking_index_synced() {
    let fixture = Fixture::start().await;
    let mut socket = fixture.connect(1).await;
    let writer = fixture.writer().await;
    report(&mut socket, 1, vec![]).await;
    fixture.blocked().await;
    // These reports stay in wire order while the first transaction waits.
    for (sequence, status) in [(2, ExecutionStatus::Running), (3, ExecutionStatus::Done)] {
        send(
            &mut socket,
            NodeFrame::Snapshot {
                snapshot: snapshot(sequence),
            },
        )
        .await;
        report(&mut socket, sequence, vec![index(status)]).await;
    }
    socket.send(Message::Ping(vec![7])).await.unwrap();
    let pong = tokio::time::timeout(Duration::from_secs(1), socket.next()).await;
    let before_commit = fixture.state.hub.views().await.remove(0);
    writer.execute("COMMIT", ()).await.unwrap();
    let acknowledged = handoff(&mut socket).await;
    assert!(
        matches!(pong, Ok(Some(Ok(Message::Pong(data)))) if data == [7]),
        "Server failed to answer Ping while report storage was blocked"
    );
    assert!(before_commit.online);
    let before = before_commit.snapshot.unwrap();
    assert!(!before.ready, "Ping must not promote an uncommitted report");
    assert_eq!(
        before.sequence, 1,
        "queued reports must not invent fresh load"
    );
    assert_eq!(before.pending_runs, 1);
    assert!(acknowledged);
    until(async || {
        fixture
            .state
            .fleet
            .index("dag-storage")
            .await
            .unwrap()
            .is_some_and(|record| record.status == ExecutionStatus::Done)
    })
    .await;
    assert_eq!(
        fixture.state.hub.views().await[0]
            .snapshot
            .as_ref()
            .unwrap()
            .sequence,
        3
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(50), socket.next())
            .await
            .is_err(),
        "subsequent reports repeated the initial handoff acknowledgement"
    );
    socket.close(None).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn disconnected_report_cannot_mark_a_reconnected_generation_ready() {
    let fixture = Fixture::start().await;
    let mut old = fixture.connect(1).await;
    let writer = fixture.writer().await;
    report(&mut old, 1, vec![]).await;
    fixture.blocked().await;
    old.close(None).await.unwrap();
    until(async || !fixture.state.hub.views().await[0].online).await;
    // The same Host can reconnect with its unchanged generation. A late
    // completion from the closed socket must not promote this new socket.
    let mut new = fixture.open(10).await;
    assert!(
        !fixture.state.hub.views().await[0]
            .snapshot
            .as_ref()
            .unwrap()
            .ready
    );
    writer.execute("COMMIT", ()).await.unwrap();
    // Register follows the old transaction through the same writer gate.
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
        "closed socket's report sent a handoff acknowledgement to its replacement"
    );
    report(&mut new, 1, vec![]).await;
    assert!(handoff(&mut new).await);
    assert!(
        fixture.state.hub.views().await[0]
            .snapshot
            .as_ref()
            .unwrap()
            .ready
    );
    new.close(None).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn invalid_report_closes_without_handoff_or_ready_promotion() {
    let fixture = Fixture::start().await;
    let mut socket = fixture.connect(1).await;
    let mut wrong = index(ExecutionStatus::Running);
    wrong.node_id = "another-node".into();
    report(&mut socket, 1, vec![wrong]).await;
    assert!(!handoff(&mut socket).await);
    until(async || !fixture.state.hub.views().await[0].online).await;
    assert!(
        !fixture.state.hub.views().await[0]
            .snapshot
            .as_ref()
            .unwrap()
            .ready
    );
    assert!(fixture
        .state
        .fleet
        .index("dag-storage")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn interleaved_load_snapshot_does_not_replace_report_watermark() {
    let fixture = Fixture::start().await;
    let mut socket = fixture.connect(1).await;
    send(
        &mut socket,
        NodeFrame::IndexReport {
            report: IndexReportEnvelope::begin(1),
        },
    )
    .await;
    send(
        &mut socket,
        NodeFrame::IndexReport {
            report: IndexReportEnvelope::batch(1, vec![index(ExecutionStatus::Running)]),
        },
    )
    .await;
    send(
        &mut socket,
        NodeFrame::Snapshot {
            snapshot: snapshot(100),
        },
    )
    .await;
    send(
        &mut socket,
        NodeFrame::IndexReport {
            report: IndexReportEnvelope::end(1),
        },
    )
    .await;
    assert!(handoff(&mut socket).await);
    assert_eq!(
        fixture.state.hub.views().await[0]
            .snapshot
            .as_ref()
            .unwrap()
            .sequence,
        100
    );
    // A newer inventory must remain eligible even though a mutation sampled
    // a later load while this earlier complete inventory was being sent.
    fixture
        .state
        .fleet
        .apply_index_report_fenced(
            "node-storage",
            &[index(ExecutionStatus::Done)],
            None,
            Some((GENERATION, 2)),
        )
        .await
        .unwrap();
    assert_eq!(
        fixture
            .state
            .fleet
            .index("dag-storage")
            .await
            .unwrap()
            .unwrap()
            .status,
        ExecutionStatus::Done
    );
    socket.close(None).await.unwrap();
}
