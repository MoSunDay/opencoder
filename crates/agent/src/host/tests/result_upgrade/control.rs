use super::*;
use serde_json::Value;

pub(super) struct Server {
    base: String,
    token: String,
    client: reqwest::Client,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    state: Arc<opencoder_control::AppState>,
}
impl Drop for Server {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}
impl Server {
    pub async fn start(host: Arc<Host>, root: &Path) -> Self {
        host.activate_host().await.unwrap();
        let work = root.join("control-work");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::write(
            work.join("opencoder.json"),
            json!({"agent":{"agents_dir":root.join("control-agents")}}).to_string(),
        )
        .unwrap();
        let state = opencoder_control::new_state(work, root.join("control-data"), None)
            .await
            .unwrap();
        let app = opencoder_control::build_app(state.clone(), Some(host.token.clone()), true);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let service: Arc<dyn NodeService> = host.clone();
        let token = host.token.clone();
        let remote = base.clone();
        let link = tokio::spawn(async move {
            opencoder_node::fleet::run(&remote, &token, service)
                .await
                .unwrap();
        });
        opencoder_control::scheduler::start(&state);
        Self {
            base,
            token: host.token.clone(),
            client: reqwest::Client::builder().no_proxy().build().unwrap(),
            tasks: vec![server, link],
            state,
        }
    }
    async fn call(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> Value {
        let mut request = self
            .client
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(&self.token);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.unwrap();
        let status = response.status();
        let value: Value = response.json().await.unwrap();
        assert!(status.is_success(), "{path}: {status}: {value}");
        value
    }
    pub async fn collect(&self, results: &[CollectedResult<'_>]) -> Vec<(String, String)> {
        let mut links = Vec::new();
        for (id, _, _, _) in results {
            tokio::time::timeout(Duration::from_secs(60), async {
                while self.state.fleet.index(id).await.unwrap().is_none() {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            })
            .await
            .unwrap();
            let todo = self
                .call(
                    reqwest::Method::POST,
                    "/api/project/todos",
                    Some(json!({"title":"historical result","draft":"read complete evidence"})),
                )
                .await;
            let path = format!(
                "/api/project/todos/{}/executions",
                todo["id"].as_str().unwrap()
            );
            self.call(
                reqwest::Method::POST,
                &path,
                Some(json!({"execution_id":id})),
            )
            .await;
            links.push((path, id.to_string()));
        }
        self.complete(&links).await;
        links
    }
    async fn complete(&self, links: &[(String, String)]) {
        tokio::time::timeout(Duration::from_secs(90), async {
            loop {
                let mut done = true;
                for (path, _) in links {
                    let value = self.call(reqwest::Method::GET, path, None).await;
                    let row = &value["assignments"][0];
                    assert!(row.get("sync_state").is_none(), "{row}");
                    assert!(row.get("result_md").is_none(), "{row}");
                    let id = row["execution_id"].as_str().unwrap();
                    let result = self
                        .call(
                            reqwest::Method::GET,
                            &format!("/api/executions/{id}/result"),
                            None,
                        )
                        .await;
                    if result["summary"].is_null() {
                        done = false;
                        continue;
                    }
                    assert!(
                        result["summary"]
                            .as_str()
                            .unwrap()
                            .contains("historical conclusion"),
                        "{result}"
                    );
                }
                if done {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .unwrap();
    }
    pub async fn refresh(&self, links: &[(String, String)]) {
        self.complete(links).await;
    }
}
