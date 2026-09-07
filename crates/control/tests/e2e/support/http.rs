//! HTTP-side harness: boots the real router (bearer auth + web assets),
//! links the scripted WS node via `opencoder_node::fleet::run`, and offers
//! reqwest helpers for JSON, raw and SSE calls.

use std::sync::Arc;
use std::time::Duration;

use opencoder_core::fleet::{ExecutionIndex, ExecutionKind, ExecutionStatus};
use opencoder_llm::{ChatStream, MockChatClient};
use opencoder_node::fleet::NodeService;
use serde_json::{json, Value};

use super::{node::MockNode, TOKEN};

pub struct Harness {
    pub base: String,
    pub state: Arc<opencoder_control::AppState>,
    pub node: Arc<MockNode>,
    pub mock_llm: Arc<MockChatClient>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    /// Keeps the temp workspace alive for the harness lifetime.
    _dir: tempfile::TempDir,
}

impl Harness {
    /// Full topology: server (auth + web assets) + one scripted WS node.
    pub async fn new() -> Arc<Self> {
        Self::new_inner(None).await
    }

    /// Same topology with an injected project store (failure-path tests).
    pub async fn with_projects(projects: Arc<dyn opencoder_store::ProjectStore>) -> Arc<Self> {
        Self::new_inner(Some(projects)).await
    }

    async fn new_inner(projects: Option<Arc<dyn opencoder_store::ProjectStore>>) -> Arc<Self> {
        let dir = tempfile::tempdir().unwrap();
        let mock_llm = Arc::new(MockChatClient::new());
        let state = match projects {
            Some(projects) => {
                opencoder_control::new_state_with_projects(
                    dir.path().join("work"),
                    dir.path().join("data"),
                    Some(mock_llm.clone() as Arc<dyn ChatStream>),
                    projects,
                )
                .await
            }
            None => {
                opencoder_control::new_state(
                    dir.path().join("work"),
                    dir.path().join("data"),
                    Some(mock_llm.clone() as Arc<dyn ChatStream>),
                )
                .await
            }
        }
        .unwrap();
        let app = opencoder_control::build_app(state.clone(), Some(TOKEN.into()), true);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let node = MockNode::new("node-e2e");
        let link = {
            let remote = base.clone();
            let service: Arc<dyn NodeService> = node.clone();
            tokio::spawn(async move {
                let _ = opencoder_node::fleet::run(&remote, TOKEN, service).await;
            })
        };
        let harness = Arc::new(Self {
            base: base.clone(),
            state: state.clone(),
            node,
            mock_llm,
            tasks: vec![server, link],
            _dir: dir,
        });
        harness.wait_online().await;
        harness
    }

    async fn wait_online(&self) {
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                assert!(!self.tasks[1].is_finished(), "node channel exited");
                if self
                    .state
                    .hub
                    .views()
                    .await
                    .iter()
                    .any(|n| n.online && n.snapshot.as_ref().is_some_and(|s| s.ready))
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("node initial sync");
    }

    /// Writes an execution index straight into the control DB (simulates an
    /// execution created in an earlier server lifetime).
    pub async fn put_index(&self, id: &str, kind: ExecutionKind, status: ExecutionStatus) {
        self.state
            .fleet
            .put_index(&ExecutionIndex {
                id: id.into(),
                created_at: opencoder_core::message::now_ms(),
                kind,
                node_id: self.node.id.clone(),
                status,
            })
            .await
            .unwrap();
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    /// JSON request with the valid bearer; returns (status, parsed body).
    pub async fn req(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> (reqwest::StatusCode, Value) {
        let resp = self.req_raw(method, path, body, Some(TOKEN)).await;
        let status = resp.status();
        let bytes = resp.bytes().await.unwrap();
        let parsed = if bytes.is_empty() {
            json!({})
        } else {
            serde_json::from_slice(&bytes).unwrap_or(json!({}))
        };
        (status, parsed)
    }

    pub async fn req_raw(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
        token: Option<&str>,
    ) -> reqwest::Response {
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let mut builder = client
            .request(method, self.url(path))
            .header("accept", "text/event-stream");
        if let Some(token) = token {
            builder = builder.bearer_auth(token);
        }
        if let Some(body) = body {
            builder = builder.json(&body);
        }
        builder.send().await.unwrap()
    }

    /// Generic raw request: no JSON auto-encoding, no implicit headers.
    /// `content_type` only applies when a body is present; extra headers are
    /// applied verbatim (e.g. `("last-event-id", "2")`). Returns
    /// (status, raw body bytes).
    #[allow(clippy::too_many_arguments)]
    pub async fn req_bytes(
        &self,
        method: reqwest::Method,
        path: &str,
        raw_body: Option<String>,
        content_type: Option<&str>,
        token: Option<&str>,
        headers: &[(&str, &str)],
    ) -> (reqwest::StatusCode, Vec<u8>) {
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let mut builder = client.request(method, self.url(path));
        if let Some(token) = token {
            builder = builder.bearer_auth(token);
        }
        if let Some(body) = raw_body {
            if let Some(content_type) = content_type {
                builder = builder.header(reqwest::header::CONTENT_TYPE, content_type);
            }
            builder = builder.body(body);
        }
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        let resp = builder.send().await.unwrap();
        let status = resp.status();
        (status, resp.bytes().await.unwrap().to_vec())
    }

    /// Reads an SSE response to stream end (the control plane closes it once
    /// the owning node reports `finished`).
    pub async fn sse_text(&self, path: &str) -> (reqwest::StatusCode, String) {
        let resp = self
            .req_raw(reqwest::Method::GET, path, None, Some(TOKEN))
            .await;
        let status = resp.status();
        (status, resp.text().await.unwrap())
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        for task in self.tasks.drain(..) {
            task.abort();
        }
    }
}
