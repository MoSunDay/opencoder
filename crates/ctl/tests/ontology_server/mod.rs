mod directories;
mod records;
mod schema;

use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc};

pub struct Server {
    root: tempfile::TempDir,
    url: String,
    pub state: Arc<opencoder_control::AppState>,
    task: tokio::task::JoinHandle<()>,
}
impl Server {
    pub async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let work = root.path().join("work");
        std::fs::create_dir(&work).unwrap();
        std::fs::write(
            work.join("opencoder.json"),
            json!({"agent":{
                "agents_dir":root.path().join("agents"),"share_dir":root.path().join("share")
            }})
            .to_string(),
        )
        .unwrap();
        std::fs::write(root.path().join("ctl.json"), "{}").unwrap();
        let state = opencoder_control::new_state(work, root.path().join("data"), None)
            .await
            .unwrap();
        let app =
            opencoder_control::build_app(state.clone(), Some("catalog-test-token".into()), false);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            root,
            url,
            state,
            task,
        }
    }
    pub async fn call(&self, args: &[&str]) -> (i32, Value) {
        let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_opencoder-cli"))
            .args([
                "--server",
                &self.url,
                "--token",
                "catalog-test-token",
                "--client-config",
            ])
            .arg(self.root.path().join("ctl.json"))
            .args(args)
            .env_remove("OPENCODER_SERVER_URL")
            .env_remove("OPENCODER_SERVER_TOKEN")
            .output()
            .await
            .unwrap();
        let code = output.status.code().unwrap();
        let bytes = if output.stdout.is_empty() {
            &output.stderr
        } else {
            &output.stdout
        };
        let value = serde_json::from_slice(bytes)
            .unwrap_or_else(|_| json!({"text":String::from_utf8_lossy(bytes)}));
        (code, value)
    }
    pub async fn ok(&self, args: &[&str]) -> Value {
        let (code, value) = self.call(args).await;
        assert_eq!(code, 0, "{args:?}: {value}");
        value
    }
    async fn write(&self, args: &[&str], body: Value) -> Value {
        let body = body.to_string();
        let mut argv = vec!["ontology"];
        argv.extend(args);
        argv.extend(["--json", &body]);
        self.ok(&argv).await
    }
    async fn reject(&self, args: &[&str], body: Value) {
        let body = body.to_string();
        let mut argv = vec!["ontology"];
        argv.extend(args);
        argv.extend(["--json", &body]);
        let (code, value) = self.call(&argv).await;
        assert_eq!(code, 4, "{args:?}: {value}");
    }
    async fn kind(&self) -> String {
        let response = self
            .write(
                &["entity-types", "create"],
                json!({"key":"component","name":"组件"}),
            )
            .await;
        response["item"]["id"].as_str().unwrap().to_owned()
    }
    async fn entity(&self, kind: &str, name: &str, directory: Option<&str>) -> Value {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);
        let request = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
        self.write(
            &["entities", "create"],
            json!({
                "request_id":format!("00000000-0000-4000-8000-{request:012x}"),
                "entity_type_id":kind,
                "name":name,"source":"测试来源","ext":"测试说明","directory_id":directory
            }),
        )
        .await["item"]
            .clone()
    }
    pub fn file(&self, name: &str, value: Value) -> PathBuf {
        let path = self.root.path().join(name);
        std::fs::write(&path, value.to_string()).unwrap();
        path
    }
}

fn id(value: &Value) -> &str {
    value["id"].as_str().unwrap()
}

fn row<'a>(list: &'a Value, id: &str) -> &'a Value {
    list["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == id)
        .unwrap_or_else(|| panic!("missing {id} in {list}"))
}

fn absent(list: &Value, id: &str) {
    assert!(
        list["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["id"] != id),
        "{id} unexpectedly present: {list}"
    );
}
impl Drop for Server {
    fn drop(&mut self) {
        self.state.lifecycle.retire();
        self.task.abort();
    }
}
