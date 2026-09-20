use base64::{engine::general_purpose::STANDARD as B64, Engine};
use opencoder_llm::MockChatClient;
use opencoder_store::{LibsqlStore, Store};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

pub struct Server {
    pub temp: tempfile::TempDir,
    pub root: PathBuf,
    pub state: Arc<opencoder_web::AppState>,
    pub url: String,
    pub client: reqwest::Client,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    pub async fn start(client: impl Into<Arc<MockChatClient>>) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("agents");
        std::fs::create_dir_all(&root).unwrap();
        let workdir = temp.path().join("work");
        std::fs::create_dir_all(workdir.join(".opencoder")).unwrap();
        std::fs::write(
            workdir.join("opencoder.json"),
            json!({"agent":{"agents_dir":root},"model":"mock/test"}).to_string(),
        )
        .unwrap();
        std::fs::write(workdir.join(".opencoder/ap.json"), r#"{"mode":"off"}"#).unwrap();
        let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
        let state = Arc::new(opencoder_web::AppState {
            config_home: None,
            brain: opencoder_web::api_brain::mock_brain(store.clone()),
            store,
            workdir,
            handles: opencoder_web::handle::new_handle_map(),
            nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
            controls: Arc::new(opencoder_web::control_state::ControlHub::new()),
            team: opencoder_web::team_state::mock(),
            project: opencoder_web::ProjectService::new(),
            client_override: Some(client.into()),
        });
        let scope = root.clone();
        let app =
            opencoder_web::build_app(state.clone(), None, false).layer(axum::middleware::from_fn(
                move |request: axum::extract::Request, next: axum::middleware::Next| {
                    let root = scope.clone();
                    async move {
                        opencoder_core::agent::scope::with_root(Some(root), next.run(request)).await
                    }
                },
            ));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            temp,
            root,
            state,
            url,
            client: reqwest::Client::builder().no_proxy().build().unwrap(),
            task,
        }
    }
    pub async fn call(&self, method: &str, path: &str, body: Option<Value>) -> (u16, Value) {
        let mut req = self
            .client
            .request(method.parse().unwrap(), format!("{}{path}", self.url));
        if let Some(body) = body {
            req = req.json(&body);
        }
        let response = req.send().await.unwrap();
        let status = response.status().as_u16();
        let raw = response.text().await.unwrap();
        (
            status,
            serde_json::from_str(&raw).unwrap_or(Value::String(raw)),
        )
    }
    pub async fn create(&self, name: &str, refs: Value) {
        let result = self
            .call(
                "POST",
                "/api/agents",
                Some(json!({"name":name,"current":refs})),
            )
            .await;
        assert_eq!(result.0, 201, "{result:?}");
    }
    pub async fn view(&self, name: &str, cat: &str) -> Value {
        let (status, view) = self
            .call("GET", &format!("/api/agents/{name}/resources/{cat}"), None)
            .await;
        assert_eq!(status, 200, "{view}");
        view
    }
    pub async fn save(&self, name: &str, cat: &str, files: Value) -> Value {
        let view = self.view(name, cat).await;
        let (status, next) = self
            .call(
                "PUT",
                &format!("/api/agents/{name}/resources/{cat}"),
                Some(json!({"baseline":view["baseline"],"files":files})),
            )
            .await;
        assert_eq!(status, 200, "{next}");
        next
    }
    pub fn scoped<T>(&self, f: impl FnOnce() -> T) -> T {
        opencoder_core::agent::scope::with_root_sync(Some(self.root.clone()), f)
    }
}
pub fn file(path: &str, bytes: impl AsRef<[u8]>) -> Value {
    json!({"path":path,"content_b64":B64.encode(bytes),"mode":384})
}
pub fn contents(view: &Value, path: &str) -> Vec<u8> {
    let file = view["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["path"] == path)
        .unwrap();
    B64.decode(file["content_b64"].as_str().unwrap()).unwrap()
}
pub fn copy_tree(source: &Path, target: &Path) {
    std::fs::create_dir_all(target).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = target.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}
