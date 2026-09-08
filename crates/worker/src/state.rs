use crate::{
    journal::Journal,
    runtime::{capacity_error, AdmissionState},
    DirectoryLayout, WorkerRuntime,
};
use anyhow::Result;
use opencoder_core::{fleet::*, Config};
use opencoder_llm::ChatStream;
use opencoder_store::{LibsqlStore, Store};
use std::{
    collections::HashMap,
    fs::File,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
use tokio::sync::{Mutex, Semaphore};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
pub struct WorkerOptions {
    pub name: String,
    pub workdir: PathBuf,
    pub data_dir: PathBuf,
    pub workflow_root: Option<PathBuf>,
    pub max_runs: Option<usize>,
    pub dag: bool,
}
#[derive(Clone)]
pub struct Worker {
    pub(crate) inner: Arc<Inner>,
}
pub(crate) struct Inner {
    pub state: Arc<opencoder_web::AppState>,
    pub client: Option<Arc<dyn ChatStream>>,
    pub layout: DirectoryLayout,
    pub registration: NodeRegistration,
    pub generation: String,
    pub sequence: AtomicU64,
    pub persistence_error: std::sync::Mutex<Option<String>>,
    pub admission_state: AdmissionState,
    pub runtime: WorkerRuntime,
    pub data_dir: PathBuf,
    pub cpu: f64,
    pub max_runs: usize,
    pub journal: Mutex<Journal>,
    pub active: Mutex<HashMap<String, CancellationToken>>,
    pub tasks: Arc<ExecutionTasks>,
    pub lifecycle_gates: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    pub slots: Arc<Semaphore>,
    pub admission: Mutex<()>,
    pub maintenance: std::sync::Mutex<HashMap<String, opencoder_session::extensions::Registration>>,
    pub _lock: File,
}
impl Worker {
    pub async fn open(options: WorkerOptions, client: Option<Arc<dyn ChatStream>>) -> Result<Self> {
        Self::open_with_runtime(options, client, WorkerRuntime::default()).await
    }

    pub async fn open_with_runtime(
        options: WorkerOptions,
        client: Option<Arc<dyn ChatStream>>,
        runtime: WorkerRuntime,
    ) -> Result<Self> {
        if options.name.trim().is_empty() || options.max_runs == Some(0) {
            anyhow::bail!("node name and positive max-runs required");
        }
        std::fs::create_dir_all(&options.data_dir)?;
        let data_dir = std::fs::canonicalize(&options.data_dir)?;
        let workflow_root = options
            .workflow_root
            .map(|path| -> std::io::Result<PathBuf> {
                if path.is_absolute() {
                    Ok(path)
                } else {
                    Ok(std::env::current_dir()?.join(path))
                }
            })
            .transpose()?;
        let layout = DirectoryLayout::new(data_dir.clone(), workflow_root)?;
        let lock = crate::migration_io::open_lock_file(&data_dir.join("node.lock"))?;
        fs2::FileExt::try_lock_exclusive(&lock)
            .map_err(|e| anyhow::anyhow!("node data directory already in use: {e}"))?;
        let admission_state = AdmissionState::load(&data_dir)?;
        opencoder_dag_runtime::sandbox::runc::cleanup_owned_containers(&[
            layout
                .checked_kind_root(ExecutionKind::Dag)?
                .join("bundles"),
            layout.checked_legacy_workflow_root()?.join("bundles"),
        ])
        .await
        .map_err(|error| anyhow::anyhow!("node-owned runc recovery failed: {error:#}"))?;
        let identity_path = data_dir.join("node-id");
        crate::migration_io::reject_symlink(&identity_path, "node identity")?;
        let id = if identity_path.exists() {
            std::fs::read_to_string(&identity_path)?.trim().to_string()
        } else {
            let id = format!("node-{}", ulid::Ulid::new());
            opencoder_core::atomic_write(&identity_path, id.as_bytes())?;
            id
        };
        if !valid_id(&id) {
            anyhow::bail!("invalid persisted node identity");
        }
        Config::load(&options.workdir)?;
        let runtime_db = data_dir.join("runtime.db");
        for path in [
            runtime_db.clone(),
            data_dir.join("runtime.db-wal"),
            data_dir.join("runtime.db-shm"),
        ] {
            crate::migration_io::reject_symlink(&path, "node runtime database")?;
        }
        let libsql = Arc::new(LibsqlStore::open(runtime_db).await?);
        let store: Arc<dyn Store> = libsql.clone();
        let project = opencoder_project::ProjectService::new();
        // 节点执行面没有 brain 运行时：brain todo 在节点上拒绝启动（由
        // 控制面预解析后再派发）。
        project
            .init(
                store.clone(),
                libsql,
                options.workdir.clone(),
                client.clone(),
                None,
            )
            .await?;
        *project.require()?.archive_root.lock().unwrap() = data_dir.join("project-runs");
        let state = Arc::new(opencoder_web::AppState {
            store: store.clone(),
            workdir: options.workdir,
            handles: opencoder_web::handle::new_handle_map(),
            nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
            controls: Arc::new(opencoder_web::control_state::ControlHub::new()),
            project,
            // These legacy HTTP contexts are never used for fleet team/brain
            // execution. The workload adapters own those explicitly below.
            team: opencoder_web::team_state::production(store.clone(), &data_dir)?,
            brain: opencoder_web::api_brain::degraded_brain(store),
            client_override: client.clone(),
        });
        let mut kinds = vec![
            ExecutionKind::Agent,
            ExecutionKind::Team,
            ExecutionKind::Todos,
            ExecutionKind::Project,
            ExecutionKind::Maintenance,
        ];
        if options.dag {
            kinds.push(ExecutionKind::Dag);
        }
        let cpu = opencoder_node::fleet::cpu::capacity();
        let max_runs = options.max_runs.unwrap_or(cpu.ceil() as usize).max(1);
        let journal = Journal::open(layout.clone())?;
        Ok(Self {
            inner: Arc::new(Inner {
                state,
                client,
                layout,
                registration: NodeRegistration {
                    protocol_version: PROTOCOL_VERSION,
                    maintenance_agent_id: format!("maintainer-{id}"),
                    id,
                    name: options.name,
                    version: env!("CARGO_PKG_VERSION").into(),
                    kinds,
                },
                generation: ulid::Ulid::new().to_string(),
                sequence: AtomicU64::new(0),
                persistence_error: std::sync::Mutex::new(None),
                admission_state,
                runtime,
                data_dir,
                cpu,
                max_runs,
                journal: Mutex::new(journal),
                active: Mutex::new(HashMap::new()),
                tasks: Arc::new(ExecutionTasks::new()),
                lifecycle_gates: Mutex::new(HashMap::new()),
                slots: Arc::new(Semaphore::new(max_runs)),
                admission: Mutex::new(()),
                maintenance: std::sync::Mutex::new(HashMap::new()),
                _lock: lock,
            }),
        })
    }
    pub async fn shutdown(&self) -> Result<()> {
        let _admission = self.inner.admission.lock().await;
        for cancel in self.inner.active.lock().await.values() {
            cancel.cancel();
        }
        opencoder_session::tools::bg::cleanup_all();
        let deadline = tokio::time::Instant::now() + self.inner.runtime.drain.cleanup_grace;
        while !self.inner.active.lock().await.is_empty() {
            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!("node shutdown timed out; unfinished work will be marked interrupted on restart");
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        self.wait_for_cleanup(deadline).await?;
        Ok(())
    }

    /// Drain naturally, interrupt leftovers, and prove owned cleanup completed.
    pub async fn drain_shutdown(&self) -> Result<()> {
        self.freeze_admission().await?;
        let natural_deadline = tokio::time::Instant::now() + self.inner.runtime.drain.natural_grace;
        while !self.naturally_quiescent().await {
            if tokio::time::Instant::now() >= natural_deadline {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        let unfinished: Vec<_> = self
            .inner
            .journal
            .lock()
            .await
            .records
            .values()
            .filter(|record| {
                crate::lifecycle::shutdown_interrupt_required(record.assignment.index.status)
            })
            .map(|record| record.assignment.index.id.clone())
            .collect();
        let mut stop_errors = Vec::new();
        for id in unfinished {
            if let Err(error) =
                crate::operations::durable_stop(self, &id, crate::lifecycle::StopIntent::Interrupt)
                    .await
            {
                stop_errors.push(format!("{id}: {error:#}"));
            }
        }
        opencoder_session::tools::bg::cleanup_all();
        let cleanup_deadline = tokio::time::Instant::now() + self.inner.runtime.drain.cleanup_grace;
        let cleanup = self.wait_for_cleanup(cleanup_deadline).await;
        if !stop_errors.is_empty() {
            anyhow::bail!(
                "node drain could not persist interrupts: {}",
                stop_errors.join("; ")
            );
        }
        cleanup
    }

    pub async fn freeze_admission(&self) -> Result<()> {
        let _gate = self.inner.admission.lock().await;
        self.inner.admission_state.freeze()?;
        opencoder_session::loop_registry::notify_change();
        Ok(())
    }

    pub async fn reopen_admission(&self) -> Result<()> {
        let _gate = self.inner.admission.lock().await;
        if let Some(error) = self.storage_error() {
            anyhow::bail!(error);
        }
        self.inner.admission_state.reopen()?;
        opencoder_session::loop_registry::notify_change();
        Ok(())
    }

    pub fn admission_open(&self) -> bool {
        self.inner.admission_state.is_open()
    }

    pub(crate) fn admission_error(&self) -> Option<String> {
        if !self.admission_open() {
            return Some("node admission is frozen".into());
        }
        self.storage_error()
    }

    fn storage_error(&self) -> Option<String> {
        match (self.inner.runtime.health)(&self.inner.data_dir) {
            Ok(capacity) => capacity_error(capacity),
            Err(error) => Some(format!("node storage health unavailable: {error:#}")),
        }
    }

    async fn naturally_quiescent(&self) -> bool {
        self.inner.active.lock().await.is_empty()
            && self.inner.tasks.active_count() == 0
            && opencoder_session::process::active_owned_processes() == 0
    }

    async fn wait_for_cleanup(&self, deadline: tokio::time::Instant) -> Result<()> {
        let (tasks, owners) = tokio::join!(
            self.inner.tasks.wait(deadline),
            opencoder_session::process::wait_for_owned_processes(deadline)
        );
        tasks?;
        owners?;
        anyhow::ensure!(
            self.inner.active.lock().await.is_empty(),
            "node shutdown completed tasks with active executions remaining"
        );
        Ok(())
    }
    pub(crate) fn next_sequence(&self) -> u64 {
        self.inner.sequence.fetch_add(1, Ordering::SeqCst) + 1
    }
    pub(crate) async fn lifecycle_gate(&self, id: &str) -> Arc<Mutex<()>> {
        self.inner
            .lifecycle_gates
            .lock()
            .await
            .entry(id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }
    pub(crate) fn configuration(&self) -> Result<Config> {
        Ok(opencoder_core::agent::scope::with_root_sync(None, || {
            Config::load(&self.inner.state.workdir)
        })?)
    }
    pub(crate) fn client(&self, config: &Config) -> Result<Arc<dyn ChatStream>> {
        if let Some(client) = &self.inner.client {
            return Ok(client.clone());
        }
        Ok(Arc::new(ConfiguredClient(config.clone())))
    }
}

pub(crate) struct ExecutionTasks {
    tracker: TaskTracker,
}

impl ExecutionTasks {
    fn new() -> Self {
        Self {
            tracker: TaskTracker::new(),
        }
    }

    pub(crate) fn spawn<F>(&self, task: F)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        drop(self.tracker.spawn(task));
    }

    async fn wait(&self, deadline: tokio::time::Instant) -> Result<()> {
        self.tracker.close();
        tokio::time::timeout_at(deadline, self.tracker.wait())
            .await
            .map_err(|_| anyhow::anyhow!("node shutdown timed out waiting for task cleanup"))
    }

    fn active_count(&self) -> usize {
        self.tracker.len()
    }
}

struct ConfiguredClient(Config);
impl ChatStream for ConfiguredClient {
    fn chat_stream(
        &self,
        request: opencoder_llm::ChatRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<opencoder_llm::LlmEvent>> {
        let ep = self.0.resolve_endpoint()?;
        opencoder_llm::ChatClient::new_with_read_timeout(
            &ep.base_url,
            &ep.api_key,
            &ep.headers,
            self.0.stream_idle_timeout(),
            self.0.network.proxy.as_deref(),
        )?
        .chat_stream(request)
    }
}

#[cfg(test)]
mod tests;
