//! Real Server/Node channel over TLS — the `wss` deployment form.
//!
//! Nodes map an `https://` remote to `wss://` (`opencoder_node::fleet::run`)
//! and verify the Server through the rustls native-roots path. This suite
//! fronts the Server's router with a real rustls terminator (self-signed CA
//! trusted via `SSL_CERT_FILE`) and proves the full channel — registration,
//! hello handshake, inventory report and live heartbeats — over TLS, plus
//! that the listener refuses plaintext.
use opencoder_core::fleet::*;
use opencoder_node::fleet::NodeService;
use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair, SanType};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::watch;

/// A CA and a 127.0.0.1 server certificate it signed: `(ca.pem, cert.pem,
/// key.pem)` PEM strings. The CA lands in `SSL_CERT_FILE`; the pair arms the
/// acceptor.
fn certificate_material() -> (String, String, String) {
    let ca_key = KeyPair::generate().unwrap();
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca = ca_params.self_signed(&ca_key).unwrap();
    let server_key = KeyPair::generate().unwrap();
    let mut server_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    server_params.subject_alt_names = vec![SanType::IpAddress(IpAddr::V4(Ipv4Addr::LOCALHOST))];
    let server = server_params.signed_by(&server_key, &ca, &ca_key).unwrap();
    (ca.pem(), server.pem(), server_key.serialize_pem())
}

fn acceptor(cert_pem: &str, key_pem: &str) -> tokio_rustls::TlsAcceptor {
    let certs: Vec<_> = rustls_pemfile::certs(&mut cert_pem.as_bytes())
        .collect::<Result<_, _>>()
        .unwrap();
    let key = rustls_pemfile::private_key(&mut key_pem.as_bytes())
        .unwrap()
        .expect("server key pem");
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .unwrap();
    tokio_rustls::TlsAcceptor::from(Arc::new(config))
}

/// axum 0.7 serves plain TCP listeners only, so drive its Router over rustls
/// streams with hyper's http1 builder (upgrades enabled for the WebSocket
/// route). This is the TLS-terminator deployment shape.
async fn serve_tls(
    app: axum::Router,
    tls: tokio_rustls::TlsAcceptor,
) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            // Transient accept errors must not retire the terminator; the
            // test's bounded timeout is the only exit for a dead listener.
            let Ok((stream, _)) = listener.accept().await else {
                continue;
            };
            let Ok(stream) = tls.accept(stream).await else {
                continue;
            };
            let service = hyper_util::service::TowerToHyperService::new(
                app.clone().into_service::<hyper::body::Incoming>(),
            );
            tokio::spawn(async move {
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
                    .with_upgrades()
                    .await;
            });
        }
    });
    addr
}

/// Minimal inventory source: ready, empty index, permissive admission.
struct TlsInventory {
    changes: watch::Sender<u64>,
}

#[async_trait::async_trait]
impl NodeService for TlsInventory {
    fn registration(&self) -> NodeRegistration {
        NodeRegistration {
            protocol_version: PROTOCOL_VERSION,
            id: "node-tls".into(),
            name: "tls".into(),
            version: "test".into(),
            maintenance_agent_id: "act".into(),
            kinds: vec![ExecutionKind::Dag],
        }
    }
    fn snapshot(&self) -> NodeSnapshot {
        NodeSnapshot {
            pending_runs: 0,
            queue_order: Default::default(),
            generation: "tls-generation".into(),
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
        Ok(Vec::new())
    }
    async fn handle(&self, operation: NodeOperation) -> RpcReply {
        assert!(matches!(operation, NodeOperation::Admission { .. }));
        RpcReply::ok(serde_json::json!({"mode":"open"}))
    }
}

// Multi-thread tests need explicit roots: thread-local config guards do not
// follow socket and catalog tasks onto Tokio worker threads.
fn isolated_work(root: &Path) -> PathBuf {
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(root.join("agents")).unwrap();
    std::fs::create_dir_all(root.join("share/todo")).unwrap();
    std::fs::write(
        work.join("opencoder.json"),
        serde_json::json!({"agent":{
            "agents_dir":root.join("agents"), "share_dir":root.join("share")
        }})
        .to_string(),
    )
    .unwrap();
    work
}

#[tokio::test]
async fn wss_channel_registers_reports_and_stays_online_over_tls() {
    let directory = tempfile::tempdir().unwrap();
    let (ca_pem, cert_pem, key_pem) = certificate_material();
    // Trust only the generated CA for the native-roots client path. This
    // binary holds a single test, so the process-global env is contained.
    let ca_path = directory.path().join("ca.pem");
    std::fs::write(&ca_path, ca_pem).unwrap();
    std::env::set_var("SSL_CERT_FILE", &ca_path);
    std::env::set_var("SSL_CERT_DIR", "");

    let state = opencoder_control::new_state(
        isolated_work(directory.path()),
        directory.path().join("data"),
        None,
    )
    .await
    .unwrap();
    let app = opencoder_control::build_app(state.clone(), Some("tls-test".into()), false);
    let addr = serve_tls(app, acceptor(&cert_pem, &key_pem)).await;

    let (changes, _) = watch::channel(0);
    let service: Arc<dyn NodeService> = Arc::new(TlsInventory { changes });
    let remote = format!("https://{addr}");
    let node = tokio::spawn(async move {
        opencoder_node::fleet::run(&remote, "tls-test", service).await
    });

    // The channel is up: registered, reported, and live over wss.
    let online = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            if state
                .hub
                .views()
                .await
                .iter()
                .any(|view| view.registration.id == "node-tls"
                    && view.online
                    && view.snapshot.as_ref().is_some_and(|s| s.ready))
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    })
    .await;
    assert!(online.is_ok(), "node never became live over wss");
    assert!(!node.is_finished());

    // The listener really is TLS-only: plaintext HTTP is dropped, so the
    // passing checks above exercised a TLS channel, not an open ws one.
    let mut plain = tokio::net::TcpStream::connect(addr).await.unwrap();
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    plain
        .write_all(b"GET /api/health HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
        .await
        .unwrap();
    let mut sink = Vec::new();
    let read = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        plain.read_to_end(&mut sink),
    )
    .await;
    // At most a TLS alert record may come back — never an HTTP response.
    let bytes = read.map(|r| r.unwrap_or(0)).unwrap_or_default();
    assert!(
        bytes <= 8 && !sink.starts_with(b"HTTP/"),
        "plaintext must be rejected by the TLS terminator, got {sink:?}"
    );

    node.abort();
}
