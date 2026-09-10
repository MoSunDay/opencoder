//! Fleet surface: node catalog fields/status derivation, per-node
//! maintenance RPC and the WS channel's bearer requirement.

use reqwest::Method;
use serde_json::{json, Value};

use crate::support::{Harness, MockNode, TOKEN};

// --- bare-server plumbing (same pattern as the ws_channel test below;
// `support/` stays untouched) ----------------------------------------------

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
/// every node link, so a topology can be torn down mid-test by aborting the
/// link task.
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
/// WS link (the hub then marks the node offline/lost).
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
/// steps, 10s cap) so snapshot/hub propagation is deterministic — never a
/// blind sleep.
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
async fn nodes_catalog_derives_busy_from_active_agent_loops() {
    let h = Harness::new().await;
    let (status, body) = h.req(Method::GET, "/api/nodes", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["nodes"][0]["status"], json!("idle"));

    // Scripted load: the fleet client uplinks a fresh Snapshot frame before
    // every operation reply (and before each index report), so a cheap
    // non-gated maintenance read deterministically triggers propagation.
    h.node.set_snapshot_opts(Some(2), None);
    h.node
        .set_maintenance("status", 200, json!({"node": {"id": "node-e2e"}}));
    let (status, body) = h
        .req(
            Method::POST,
            "/api/nodes/node-e2e/maintenance",
            Some(json!({"action": "status"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");

    let node = poll_json(&h.base, Method::GET, "/api/nodes", None, |status, body| {
        (status.as_u16() == 200 && body["nodes"][0]["status"] == json!("busy"))
            .then(|| body["nodes"][0].clone())
    })
    .await;
    assert_eq!(node["online"], json!(true));
    assert_eq!(node["snapshot"]["active_agent_loops"], json!(2));
}

#[tokio::test]
async fn nodes_catalog_derives_lost_when_the_node_link_dies() {
    let server = bare_server().await;
    let (_node, link) = spawn_node(&server.base, "node-lost");
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

    // Aborting the link task severs the WS channel; the hub detaches the node
    // and the catalog downgrades it to lost.
    link.abort();
    let node = poll_json(
        &server.base,
        Method::GET,
        "/api/nodes",
        None,
        |status, body| {
            (status.as_u16() == 200
                && body["nodes"][0]["online"] == json!(false)
                && body["nodes"][0]["status"] == json!("lost"))
            .then(|| body["nodes"][0].clone())
        },
    )
    .await;
    assert_eq!(node["id"], json!("node-lost"));
    assert!(node["last_seen_at"].as_i64().unwrap_or(0) > 0);
}

#[tokio::test]
async fn maintenance_mutations_are_admission_gated_reads_are_not() {
    let h = Harness::new().await;
    h.node.set_maintenance("ask", 200, json!({"answer": 42}));
    h.node
        .set_maintenance("configure", 200, json!({"applied": true}));
    h.node
        .set_maintenance("status", 200, json!({"node": {"id": "node-e2e"}}));

    let (status, _) = h.req(Method::POST, "/api/admin/drain", None).await;
    assert_eq!(status, 200);

    // Only ask/configure mutate node state, so a frozen control plane refuses
    // exactly those before any node round-trip.
    for action in ["ask", "configure"] {
        let (status, body) = h
            .req(
                Method::POST,
                "/api/nodes/node-e2e/maintenance",
                Some(json!({"action": action, "input": {"x": 1}})),
            )
            .await;
        assert_eq!(status, 503, "{action}: {body}");
        assert_eq!(body["error"], json!("server admission is frozen"));
    }

    // Read-only status passes the admission gate while frozen.
    let (status, body) = h
        .req(
            Method::POST,
            "/api/nodes/node-e2e/maintenance",
            Some(json!({"action": "status"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["node"]["id"], json!("node-e2e"));
}

#[tokio::test]
async fn nodes_catalog_lists_the_connected_node() {
    let h = Harness::new().await;
    let (status, body) = h.req(Method::GET, "/api/nodes", None).await;
    assert_eq!(status, 200, "{body}");
    let nodes = body["nodes"].as_array().expect("nodes array");
    assert_eq!(nodes.len(), 1);
    let node = &nodes[0];
    assert_eq!(node["id"], json!("node-e2e"));
    assert_eq!(node["status"], json!("idle"));
    assert_eq!(node["online"], json!(true));
    assert_eq!(node["maintenance_agent_id"], json!("act"));
    assert!(node["last_seen_at"].as_i64().unwrap_or(0) > 0);
    assert_eq!(node["snapshot"]["cpu_capacity"], json!(4.0));
    assert_eq!(node["snapshot"]["ready"], json!(true));
    assert_eq!(
        node["kinds"],
        json!(["agent", "dag", "team", "todos", "project", "operator"])
    );
}

#[tokio::test]
async fn maintenance_proxies_configured_node_replies() {
    let h = Harness::new().await;
    h.node.set_maintenance(
        "status",
        200,
        json!({"node": {"id": "node-e2e"}, "snapshot": {"ready": true}}),
    );
    h.node.set_maintenance(
        "executions",
        200,
        json!({"executions": [{"id": "agent-x"}]}),
    );
    let (status, body) = h
        .req(
            Method::POST,
            "/api/nodes/node-e2e/maintenance",
            Some(json!({"action": "status"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["node"]["id"], json!("node-e2e"));

    let (status, body) = h
        .req(
            Method::POST,
            "/api/nodes/node-e2e/maintenance",
            Some(json!({"action": "executions", "input": {}})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["executions"][0]["id"], json!("agent-x"));

    // Unknown operations surface the node's 400 verbatim.
    let (status, body) = h
        .req(
            Method::POST,
            "/api/nodes/node-e2e/maintenance",
            Some(json!({"action": "bogus"})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"], json!("unknown maintenance operation"));

    // Unknown node id: the hub rejects before any node sees the call.
    let (status, body) = h
        .req(
            Method::POST,
            "/api/nodes/node-ghost/maintenance",
            Some(json!({"action": "status"})),
        )
        .await;
    assert_eq!(status, 503, "{body}");
    assert!(
        body["error"].as_str().unwrap_or("").contains("offline"),
        "{body}"
    );
}

#[tokio::test]
async fn ws_channel_rejects_wrong_bearer_token() {
    // Server with token A; a node presenting token B must never register.
    let dir = tempfile::tempdir().unwrap();
    let state =
        opencoder_control::new_state(dir.path().join("work"), dir.path().join("data"), None)
            .await
            .unwrap();
    let app = opencoder_control::build_app(state.clone(), Some(TOKEN.into()), false);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let node = MockNode::new("node-refused");
    let link = {
        let service: std::sync::Arc<dyn opencoder_node::fleet::NodeService> = node.clone();
        let remote = base.clone();
        tokio::spawn(async move {
            let _ = opencoder_node::fleet::run(&remote, "wrong-token", service).await;
        })
    };
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    let views = state.hub.views().await;
    assert!(
        views.iter().all(|v| !v.online),
        "wrong-token node must not register: {views:?}"
    );
    link.abort();
    server.abort();
}
