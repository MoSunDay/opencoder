//! Fleet admin edge cases beyond the happy drain cycle: unacknowledged node
//! freezes, reopen refusals (node veto, zero online nodes) and offline-node
//! reporting on a bare topology (server without the harness's scripted node).

use reqwest::Method;
use serde_json::{json, Value};

use crate::support::{Harness, MockNode, TOKEN};

// --- bare-server plumbing (support/ stays untouched) ------------------------

struct BareServer {
    base: String,
    server: tokio::task::JoinHandle<()>,
    /// Keeps the temp workspace alive for the server lifetime.
    _dir: tempfile::TempDir,
}

impl Drop for BareServer {
    fn drop(&mut self) {
        self.server.abort();
    }
}

/// Boots the real router (bearer auth, no scripted node): the caller owns
/// every node link, so the topology can be torn down mid-test by aborting a
/// link task or left empty.
async fn bare_server() -> BareServer {
    let dir = tempfile::tempdir().unwrap();
    let state =
        opencoder_control::new_state(dir.path().join("work"), dir.path().join("data"), None)
            .await
            .unwrap();
    let app = opencoder_control::build_app(state, Some(TOKEN.into()), false);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    BareServer {
        base,
        server,
        _dir: dir,
    }
}

/// Connects a scripted node to `base`; aborting the returned task severs the
/// WS link (the hub then marks the node offline).
fn spawn_node(base: &str, id: &str) -> (std::sync::Arc<MockNode>, tokio::task::JoinHandle<()>) {
    let node = MockNode::new(id);
    let service: std::sync::Arc<dyn opencoder_node::fleet::NodeService> = node.clone();
    let remote = base.to_owned();
    let task = tokio::spawn(async move {
        let _ = opencoder_node::fleet::run(&remote, TOKEN, service).await;
    });
    (node, task)
}

/// Authorized JSON call against any base URL (harness or bare server).
async fn api_json(
    base: &str,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> (reqwest::StatusCode, Value) {
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let mut builder = client
        .request(method, format!("{base}{path}"))
        .bearer_auth(TOKEN);
    if let Some(body) = &body {
        builder = builder.json(body);
    }
    let resp = builder.send().await.unwrap();
    let status = resp.status();
    let bytes = resp.bytes().await.unwrap();
    let parsed = serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({}));
    (status, parsed)
}

/// Polls an authorized JSON endpoint until `extract` yields a value (100ms
/// steps, 10s cap) so hub propagation is deterministic — never a blind sleep.
async fn poll_json<T>(
    base: &str,
    method: Method,
    path: &str,
    body: Option<Value>,
    extract: impl Fn(&reqwest::StatusCode, &Value) -> Option<T>,
) -> T {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let (status, parsed) = api_json(base, method.clone(), path, body.clone()).await;
        if let Some(value) = extract(&status, &parsed) {
            return value;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timeout polling {path}: status={status} body={parsed}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn freeze_unacknowledged_by_node_leaves_cluster_not_drained() {
    let h = Harness::new().await;
    // The node acks 200 but claims it stayed open — not a frozen ack, so the
    // cluster aggregate cannot report drained even though the server froze.
    h.node.set_admission_reply(
        "freeze",
        200,
        json!({"mode": "open", "active_runs": 0, "owned_processes": 0}),
    );
    let (status, body) = h.req(Method::POST, "/api/admin/drain", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["server"]["mode"], json!("frozen"));
    assert_eq!(body["nodes"][0]["body"]["mode"], json!("open"));
    assert_eq!(body["drained"], json!(false));
    // The scripted override also skips the node's own freeze side effect.
    assert!(h.node.open.load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test]
async fn reopen_refused_by_node_keeps_cluster_frozen() {
    let h = Harness::new().await;
    let (status, _) = h.req(Method::POST, "/api/admin/drain", None).await;
    assert_eq!(status, 200);
    assert!(!h.node.open.load(std::sync::atomic::Ordering::SeqCst));

    // One rejecting node vetoes the reopen; the server must stay frozen.
    h.node
        .set_admission_reply("reopen", 409, json!({"error": "no"}));
    let (status, body) = h.req(Method::DELETE, "/api/admin/drain", None).await;
    assert_eq!(status, 503, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .contains("rejected admission reopen"),
        "{body}"
    );
    assert_eq!(body["nodes"][0]["status"], json!(409));
    assert_eq!(body["nodes"][0]["body"]["error"], json!("no"));

    let (status, body) = h.req(Method::GET, "/api/admin/drain", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["server"]["mode"], json!("frozen"));
    assert!(!h.node.open.load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test]
async fn drain_status_reports_offline_nodes() {
    let server = bare_server().await;
    let (_node, link) = spawn_node(&server.base, "node-gone");
    poll_json(
        &server.base,
        Method::GET,
        "/api/nodes",
        None,
        |status, body| {
            (status.as_u16() == 200
                && body["nodes"]
                    .as_array()
                    .is_some_and(|nodes| nodes.len() == 1 && nodes[0]["online"] == json!(true)))
            .then_some(())
        },
    )
    .await;

    // Sever the link; the drain status must name the node in offline_nodes
    // and fan the status call out to nobody.
    link.abort();
    poll_json(
        &server.base,
        Method::GET,
        "/api/nodes",
        None,
        |status, body| {
            (status.as_u16() == 200 && body["nodes"][0]["online"] == json!(false)).then_some(())
        },
    )
    .await;
    let (status, body) = api_json(&server.base, Method::GET, "/api/admin/drain", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["offline_nodes"], json!(["node-gone"]));
    assert_eq!(body["nodes"], json!([]));
    // Server was never frozen, so the aggregate cannot be drained.
    assert_eq!(body["server"]["mode"], json!("open"));
    assert_eq!(body["drained"], json!(false));
}

#[tokio::test]
async fn reopen_with_zero_online_nodes_is_refused_and_stays_frozen() {
    let server = bare_server().await;
    // Freezing works with no nodes attached at all …
    let (status, body) = api_json(&server.base, Method::POST, "/api/admin/drain", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["server"]["mode"], json!("frozen"));
    assert_eq!(body["drained"], json!(true));

    // … but reopening needs at least one online node to verify.
    let (status, body) = api_json(&server.base, Method::DELETE, "/api/admin/drain", None).await;
    assert_eq!(status, 503, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .contains("no online node"),
        "{body}"
    );

    let (status, body) = api_json(&server.base, Method::GET, "/api/admin/drain", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["server"]["mode"], json!("frozen"));
}
