use opencoder_dag::{DagClaimedRun, DagEventBatch, DagEventIn, DagStatusReport};
use opencoder_dag_runtime::{ExecDeps, RunDeps};
use opencoder_llm::{ChatRequest, ChatStream, LlmEvent};
use opencoder_node::uplink::{LocalDagPersistence, Uplink};
use opencoder_store::{LibsqlStore, Store};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{Arc, LazyLock, Mutex},
    time::Duration,
};

#[path = "../support/container.rs"]
mod container;
#[path = "../support/model.rs"]
mod model;

#[derive(Default)]
pub struct Events(pub Mutex<Vec<DagEventIn>>);
#[async_trait::async_trait]
impl LocalDagPersistence for Events {
    async fn events(&self, batch: &DagEventBatch) -> anyhow::Result<()> {
        self.0.lock().unwrap().extend(batch.events.clone());
        Ok(())
    }
    async fn status(&self, _: &DagStatusReport) -> anyhow::Result<()> {
        Ok(())
    }
}

pub struct Fixture {
    container: container::ContainerFixture,
    bridges: Mutex<Vec<model::ModelBridge>>,
    pub tmp: tempfile::TempDir,
    pub root: PathBuf,
    pub events: Arc<Events>,
    pub store: Arc<dyn Store>,
    pub config: opencoder_core::Config,
    // Each scenario still runs its own concurrent instances. Keep independent
    // scenarios from spending another scenario's deadlock budget on fsyncs.
    _permit: tokio::sync::OwnedSemaphorePermit,
}
impl Fixture {
    pub async fn new() -> Self {
        static SCENARIO: LazyLock<Arc<tokio::sync::Semaphore>> =
            LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(1)));
        let permit = SCENARIO.clone().acquire_owned().await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("workflow");
        let mut config = opencoder_core::Config::default();
        let container = container::ContainerFixture::open(tmp.path());
        container.configure(&mut config);
        config.agent.agents_dir = Some(tmp.path().join("agents"));
        std::fs::create_dir_all(config.agent.agents_dir.as_ref().unwrap()).unwrap();
        Self {
            container,
            bridges: Mutex::new(vec![]),
            tmp,
            root,
            events: Arc::new(Events::default()),
            store: Arc::new(LibsqlStore::open_memory().await.unwrap()),
            config,
            _permit: permit,
        }
    }
    pub fn run(&self, steps: Value, input: Value) -> DagClaimedRun {
        let run = DagClaimedRun {
            run_id: ulid::Ulid::new().to_string(),
            dag_id: "dynamic-test".into(),
            created_at: 0,
            spec: serde_json::from_value(json!({"name":"dynamic-test","steps":steps})).unwrap(),
        };
        std::fs::create_dir_all(self.root.join(&run.run_id)).unwrap();
        std::fs::write(
            self.root.join(&run.run_id).join("input.json"),
            serde_json::to_vec(&input).unwrap(),
        )
        .unwrap();
        run
    }
    pub fn deps(&self, client: Arc<dyn ChatStream>) -> RunDeps {
        let bridge = model::ModelBridge::start(client.clone());
        let mut config = self.config.clone();
        bridge.configure(&mut config);
        self.bridges.lock().unwrap().push(bridge);
        RunDeps {
            uplink: Arc::new(Uplink::for_local_dag(self.events.clone())),
            workflow_root: self.root.clone(),
            exec: ExecDeps {
                store: self.store.clone(),

                workdir: self.tmp.path().into(),
                config,
            },
        }
    }
    pub fn json(&self, run: &DagClaimedRun, path: &str) -> Value {
        serde_json::from_slice(&std::fs::read(self.root.join(&run.run_id).join(path)).unwrap())
            .unwrap()
    }
    pub fn text(&self, run: &DagClaimedRun, path: &str) -> String {
        std::fs::read_to_string(self.root.join(&run.run_id).join(path)).unwrap()
    }
    pub fn binary(&self, name: &str, source: &str) {
        let directory = self.tmp.path().join("compile");
        std::fs::create_dir_all(&directory).unwrap();
        let source_path = directory.join(format!("{name}.c"));
        let output_path = directory.join(name);
        std::fs::write(&source_path, source).unwrap();
        let result = std::process::Command::new("cc")
            .args(["-O2", "-static"])
            .arg(source_path)
            .arg("-o")
            .arg(&output_path)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        opencoder_dag_binary::save_binary_version(
            &self.container.pool,
            name,
            "fixture",
            &std::fs::read(output_path).unwrap(),
        )
        .unwrap();
    }
}

pub fn dynamic_agent() -> Value {
    json!({"name":"process","kind":{"type":"dynamic","source":{"type":"input","pointer":"/items"},
        "template":{"type":"agent","prompt":"common-prompt","how_append":"common-how"}}})
}

pub type ResponseFn = dyn Fn(&str) -> (Duration, Result<String, String>) + Send + Sync;
pub struct Scripted {
    pub requests: Mutex<Vec<String>>,
    pub respond: Arc<ResponseFn>,
}
impl Scripted {
    pub fn new(
        f: impl Fn(&str) -> (Duration, Result<String, String>) + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            requests: Mutex::new(vec![]),
            respond: Arc::new(f),
        })
    }
}
impl ChatStream for Scripted {
    fn chat_stream(
        &self,
        request: ChatRequest,
    ) -> anyhow::Result<tokio::sync::mpsc::Receiver<LlmEvent>> {
        let text = request
            .messages
            .iter()
            .map(|m| m.text())
            .collect::<Vec<_>>()
            .join("\n");
        self.requests.lock().unwrap().push(text.clone());
        let (delay, response) = (self.respond)(&text);
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            let event = match response {
                Ok(text) => LlmEvent::Completed {
                    text,
                    tool_calls: vec![],
                    usage: None,
                },
                Err(message) => LlmEvent::Error(message),
            };
            let _ = tx.send(event).await;
        });
        Ok(rx)
    }
    fn backend(&self) -> &'static str {
        "mock"
    }
}

pub const ARGV_C: &str = r#"
#include <stdio.h>
#include <string.h>
int main(int count, char **arguments) {
    for (int index=0; index<count; index++) fwrite(arguments[index], strlen(arguments[index])+1, 1, stdout);
    return 0;
}
"#;
