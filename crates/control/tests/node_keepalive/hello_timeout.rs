//! Server-side Hello gating: a silent or identity-mismatched socket closes.
use futures::{SinkExt, StreamExt};
use opencoder_core::fleet::*;
use std::{sync::Arc, time::Duration};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
const TOKEN: &str = "hello-timeout-test";

struct Fixture {
    state: Arc<opencoder_control::AppState>,
    remote: String,
    server: tokio::task::JoinHandle<()>,
    _root: tempfile::TempDir,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
impl Fixture {
    async fn start() -> Self {
        let root = tempfile::tempdir().unwrap();
        let _scope = opencoder_core::config::scoped_config_home(root.path().join("config"));
        let state = opencoder_control::new_state(
            super::isolated_work(root.path()),
            root.path().join("data"),
            None,
        )
        .await
        .unwrap();
        let app = opencoder_control::build_app(state.clone(), Some(TOKEN.into()), false);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let remote = format!(
            "ws://{}/api/nodes/channel?node_id=node-hello",
            listener.local_addr().unwrap()
        );
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            state,
            remote,
            server,
            _root: root,
        }
    }
    /// Authenticated raw socket; the caller decides whether (and what) to send.
    async fn connect(&self) -> Socket {
        let mut request = self.remote.clone().into_client_request().unwrap();
        request
            .headers_mut()
            .insert("authorization", format!("Bearer {TOKEN}").parse().unwrap());
        let (socket, _) = connect_async(request).await.unwrap();
        socket
    }
}

fn hello(registration_id: &str) -> NodeFrame {
    NodeFrame::Hello {
        registration: NodeRegistration {
            id: registration_id.into(),
            name: "hello-fixture".into(),
            version: "test".into(),
            protocol_version: PROTOCOL_VERSION,
            maintenance_agent_id: "maintenance-hello".into(),
            kinds: vec![ExecutionKind::Dag],
        },
        snapshot: NodeSnapshot {
            generation: "hello-generation".into(),
            sequence: 1,
            cpu_capacity: 2.0,
            active_agent_loops: 0,
            active_runs: 0,
            pending_runs: 0,
            max_runs: 2,
            queue_order: Default::default(),
            ready: true,
            resource_error: None,
        },
    }
}

/// Waits for the server to terminate the socket. Only close/transport-end
/// frames are legal; any payload proves the socket survived gating.
async fn closed_without_payload(socket: &mut Socket) -> std::time::Duration {
    let start = std::time::Instant::now();
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(frame) = socket.next().await {
            match frame {
                Ok(Message::Close(_)) => continue,
                Err(_) => break,
                other => panic!("no server frame may precede the close: {other:?}"),
            }
        }
    })
    .await
    .expect("gated socket must close within the bound");
    start.elapsed()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn missing_hello_closes_the_socket_after_the_injected_timeout() {
    let fixture = Fixture::start().await;

    // Silent socket: the injected 1s Hello deadline, not an immediate reset,
    // must be what terminates it. The knob is per state, so the shortened
    // deadline cannot leak into any other test's control state.
    opencoder_control::transport::set_hello_timeout_for_tests(&fixture.state, 1);
    let mut silent = fixture.connect().await;
    let elapsed = closed_without_payload(&mut silent).await;
    assert!(
        elapsed >= Duration::from_millis(800),
        "socket closed after {elapsed:?}, before the injected Hello deadline"
    );
    assert!(
        fixture.state.fleet.nodes().await.unwrap().is_empty(),
        "a silent socket must never register a node"
    );

    // Mismatched identity under the production default deadline: the
    // authenticated query node_id must match the Hello registration id, and
    // violations close the socket promptly without waiting for any deadline.
    let mut mismatched = fixture.connect().await;
    mismatched
        .send(Message::Text(
            serde_json::to_string(&hello("node-hello-mismatched")).unwrap(),
        ))
        .await
        .unwrap();
    let elapsed = closed_without_payload(&mut mismatched).await;
    assert!(
        elapsed < Duration::from_secs(2),
        "identity mismatch must close promptly, took {elapsed:?}"
    );
    assert!(
        fixture.state.fleet.nodes().await.unwrap().is_empty(),
        "a mismatched identity must never register a node"
    );
}
