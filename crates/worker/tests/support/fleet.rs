use super::{native, worker};
use opencoder_core::fleet::{RpcReply, MAX_FRAME_BYTES};
use opencoder_llm::ChatStream;
use opencoder_node::fleet::NodeService;
use opencoder_worker::Worker;
use serde_json::Value;
use std::{sync::Arc, time::Duration};

pub struct Fleet {
    _dag_environments: Vec<(
        native::container::ContainerFixture,
        native::model::ModelBridge,
    )>,
    pub url: String,
    pub state: Arc<opencoder_control::AppState>,
    pub nodes: Vec<Worker>,
    app: axum::Router,
    server: tokio::task::JoinHandle<()>,
    channels: Vec<tokio::task::JoinHandle<anyhow::Result<()>>>,
    client: Arc<dyn ChatStream>,
    _config: opencoder_core::config::ScopedConfigHome,
    _dir: tempfile::TempDir,
}
impl Fleet {
    pub async fn new(count: usize, client: Arc<dyn ChatStream>) -> Self {
        Self::new_with_ui(count, client, false).await
    }
    pub async fn new_with_ui(count: usize, client: Arc<dyn ChatStream>, ui: bool) -> Self {
        Self::configured(count, client, ui, |_| {}).await
    }

    pub async fn new_with_agents(
        count: usize,
        client: Arc<dyn ChatStream>,
        setup: impl FnOnce(&std::path::Path),
    ) -> Self {
        Self::configured(count, client, false, setup).await
    }

    async fn configured(
        count: usize,
        client: Arc<dyn ChatStream>,
        ui: bool,
        setup: impl FnOnce(&std::path::Path),
    ) -> Self {
        let dir = tempfile::tempdir().unwrap();
        // All Fleet fixtures run on current-thread runtimes. Keep host agent
        // pools and credentials outside the fixture for its entire lifetime,
        // including requests served by spawned node-channel tasks.
        let config = opencoder_core::config::scoped_config_home(dir.path().join("config-home"));
        // Match the real Brain model's window for complete multi-layer evidence.
        let config_path = opencoder_core::Config::global_config_path().unwrap();
        std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        std::fs::write(config_path, r#"{"context_limit":1000000}"#).unwrap();
        setup(&opencoder_core::agent::agents_dir().unwrap());
        let state = opencoder_control::new_state(
            dir.path().join("server-work"),
            dir.path().join("server"),
            Some(client.clone()),
        )
        .await
        .unwrap();
        let app = opencoder_control::build_app(state.clone(), None, ui);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let serve_app = app.clone();
        let server = tokio::spawn(async move {
            axum::serve(listener, serve_app).await.unwrap();
        });
        let mut nodes = vec![];
        let mut channels = vec![];
        let mut dag_environments = vec![];
        for i in 0..count {
            dag_environments.push(native::environment(
                &dir.path().join(format!("n{i}")),
                client.clone(),
            ));
            let node = worker(&dir.path().join(format!("n{i}")), client.clone()).await;
            let service: Arc<dyn NodeService> = Arc::new(node.clone());
            let url = url.clone();
            channels.push(tokio::spawn(async move {
                opencoder_node::fleet::run(&url, "test", service).await
            }));
            nodes.push(node);
        }
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            // Scheduling (select_node) requires a snapshot with ready=true;
            // waiting for online alone races the first submit into 503.
            while state
                .hub
                .views()
                .await
                .iter()
                .filter(|n| n.online && n.snapshot.as_ref().is_some_and(|s| s.ready))
                .count()
                != count
            {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        Self {
            _dag_environments: dag_environments,
            url,
            state,
            nodes,
            app,
            server,
            channels,
            client,
            _config: config,
            _dir: dir,
        }
    }
    /// Replace only Control; the Workers and their channel loops keep running.
    pub async fn stop_server(&mut self) {
        self.state.lifecycle.retire();
        self.state.lifecycle.retire_channels();
        self.server.abort();
        let _ = (&mut self.server).await;
    }

    pub async fn restart_server(&mut self) {
        let state = opencoder_control::new_state(
            self._dir.path().join("server-work"),
            self._dir.path().join("server"),
            Some(self.client.clone()),
        )
        .await
        .unwrap();
        let app = opencoder_control::build_app(state.clone(), None, false);
        let address = reqwest::Url::parse(&self.url).unwrap();
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", address.port().unwrap()))
            .await
            .unwrap();
        let serve_app = app.clone();
        self.server = tokio::spawn(async move {
            axum::serve(listener, serve_app).await.unwrap();
        });
        self.state = state;
        self.app = app;
        tokio::time::timeout(Duration::from_secs(15), async {
            while self
                .state
                .hub
                .views()
                .await
                .iter()
                .filter(|node| node.online && node.snapshot.as_ref().is_some_and(|s| s.ready))
                .count()
                != self.nodes.len()
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("node did not reconnect to restarted server");
    }
    pub async fn call(&self, method: &str, path: &str, body: Value) -> RpcReply {
        use tower::ServiceExt;
        let body = if body.is_null() {
            String::new()
        } else {
            body.to_string()
        };
        let response = self
            .app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method(method)
                    .uri(path)
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status().as_u16();
        let bytes = axum::body::to_bytes(response.into_body(), MAX_FRAME_BYTES)
            .await
            .unwrap();
        RpcReply {
            status,
            body: serde_json::from_slice(&bytes).unwrap(),
        }
    }
    pub async fn response(&self, method: &str, path: &str) -> axum::response::Response {
        use tower::ServiceExt;
        self.app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method(method)
                    .uri(path)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
    }
    pub fn root(&self) -> &std::path::Path {
        self._dir.path()
    }
    pub async fn shutdown(&self) {
        for node in &self.nodes {
            node.shutdown().await.unwrap();
        }
    }
    pub async fn disconnect(&self, index: usize) {
        self.channels[index].abort();
        let node_id = self.nodes[index].registration().id;
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if self
                    .state
                    .hub
                    .views()
                    .await
                    .iter()
                    .find(|node| node.registration.id == node_id)
                    .is_some_and(|node| !node.online)
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }
}
impl Drop for Fleet {
    fn drop(&mut self) {
        for channel in &self.channels {
            channel.abort();
        }
        self.server.abort();
    }
}
