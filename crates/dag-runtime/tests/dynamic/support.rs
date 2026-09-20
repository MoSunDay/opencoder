use opencoder_dag::{DagClaimedRun, DagEventBatch, DagEventIn, DagStatusReport};
use opencoder_dag_runtime::{ExecDeps, RunDeps};
use opencoder_llm::{ChatRequest, ChatStream, LlmEvent};
use opencoder_node::uplink::{LocalDagPersistence, Uplink};
use opencoder_store::{LibsqlStore, Store};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

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
    pub tmp: tempfile::TempDir,
    pub root: PathBuf,
    pub events: Arc<Events>,
    pub store: Arc<dyn Store>,
    pub config: opencoder_core::Config,
}
impl Fixture {
    pub async fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("workflow");
        let mut config = opencoder_core::Config::default();
        config.agent.agents_dir = Some(tmp.path().join("agents"));
        Self {
            tmp,
            root,
            events: Arc::new(Events::default()),
            store: Arc::new(LibsqlStore::open_memory().await.unwrap()),
            config,
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
        RunDeps {
            uplink: Arc::new(Uplink::for_local_dag(self.events.clone())),
            workflow_root: self.root.clone(),
            exec: ExecDeps {
                store: self.store.clone(),
                client,
                workdir: self.tmp.path().into(),
                config: self.config.clone(),
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
    pub fn module(&self, run: &DagClaimedRun, name: &str, wat: &str) {
        std::fs::write(
            self.root.join(&run.run_id).join(name),
            wat::parse_str(wat).unwrap(),
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

pub const ARGV_WAT: &str = r#"(module
(import "wasi_snapshot_preview1" "args_sizes_get" (func $sizes (param i32 i32) (result i32)))
(import "wasi_snapshot_preview1" "args_get" (func $args (param i32 i32) (result i32)))
(import "wasi_snapshot_preview1" "fd_write" (func $write (param i32 i32 i32 i32) (result i32)))
(memory (export "memory") 1)
(func (export "_start")
(drop (call $sizes (i32.const 0) (i32.const 4)))
(drop (call $args (i32.const 8) (i32.const 4096)))
(i32.store (i32.const 2048) (i32.const 4096))
(i32.store (i32.const 2052) (i32.load (i32.const 4)))
(drop (call $write (i32.const 1) (i32.const 2048) (i32.const 1) (i32.const 2056)))))"#;
