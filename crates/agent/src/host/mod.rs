mod admission;
mod api;
mod client;
pub mod config;
mod deployment;
mod lifecycle;
pub mod runtime;
mod service;
#[cfg(test)]
mod tests;

use anyhow::{ensure, Result};
use opencoder_core::fleet::*;
use opencoder_node::fleet::NodeService;
use opencoder_store::fleet::FleetStore;
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, RwLock,
    },
};

pub struct Host {
    pub data_dir: std::path::PathBuf,
    pub store: Arc<FleetStore>,
    pub registration: NodeRegistration,
    pub token: String,
    pub client: reqwest::Client,
    creations: admission::Creations,
    pub snapshot: RwLock<NodeSnapshot>,
    pub sequence: AtomicU64,
    pub changes: tokio::sync::watch::Sender<u64>,
    pub instance: String,
    pub retiring: AtomicBool,
    pub report_gate: tokio::sync::Mutex<()>,
    pub port: AtomicU64,
}

impl Host {
    pub async fn open(
        data: &Path,
        name: String,
        token: String,
        max_runs: usize,
    ) -> Result<Arc<Self>> {
        ensure!(
            data.is_absolute() && max_runs > 0,
            "host requires absolute data directory and positive capacity"
        );
        std::fs::create_dir_all(data)?;
        let store = Arc::new(FleetStore::open(&data.join("host.db")).await?);
        let _lock = store.request_lock("host-init", "identity").await?;
        let identity = data.join("node-id");
        let id = match std::fs::read_to_string(&identity) {
            Ok(id) => id.trim().to_string(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let id = format!("node-{}", ulid::Ulid::new());
                opencoder_core::atomic_write(&identity, id.as_bytes())?;
                id
            }
            Err(error) => return Err(error.into()),
        };
        ensure!(valid_id(&id), "invalid persisted node identity");
        store.initialize_capacity(max_runs).await?;
        let capacity = store.capacity().await?;
        let (changes, _) = tokio::sync::watch::channel(0);
        let host = Arc::new(Self {
            data_dir: data.to_path_buf(),
            store,
            registration: NodeRegistration {
                protocol_version: PROTOCOL_VERSION,
                maintenance_agent_id: format!("maintainer-{id}"),
                id,
                name,
                version: opencoder_core::version::VERSION_LONG.into(),
                kinds: vec![
                    ExecutionKind::Brain,
                    ExecutionKind::Agent,
                    ExecutionKind::Dag,
                    ExecutionKind::Team,
                    ExecutionKind::Todos,
                    ExecutionKind::Project,
                    ExecutionKind::Maintenance,
                    ExecutionKind::Operator,
                ],
            },
            token,
            client: reqwest::Client::builder().no_proxy().build()?,
            creations: admission::Creations::default(),
            snapshot: RwLock::new(NodeSnapshot {
                generation: String::new(),
                sequence: 0,
                cpu_capacity: opencoder_node::fleet::cpu::capacity(),
                active_agent_loops: 0,
                active_runs: capacity.running,
                pending_runs: capacity.queued,
                max_runs: capacity.max_runs,
                queue_order: QueueOrder::Fifo,
                ready: false,
                resource_error: None,
            }),
            sequence: AtomicU64::new(0),
            changes,
            instance: ulid::Ulid::new().to_string(),
            retiring: AtomicBool::new(false),
            report_gate: tokio::sync::Mutex::new(()),
            port: AtomicU64::new(0),
        });
        host.sync_inventory().await?;
        Ok(host)
    }

    pub async fn activate_host(&self) -> Result<u64> {
        self.sync_inventory().await?;
        let _lock = self.store.request_lock("host-epoch", "active").await?;
        let previous = self
            .store
            .definition("host", "current")
            .await?
            .unwrap_or_default();
        let epoch = if previous["instance"] == self.instance {
            previous["epoch"].as_u64().unwrap_or(0)
        } else {
            previous["epoch"]
                .as_u64()
                .unwrap_or(0)
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("host epoch exhausted"))?
        };
        self.snapshot.write().unwrap().generation = format!("host-{epoch:020}-{}", self.instance);
        self.store
            .put_definition(
                "host",
                "current",
                &serde_json::json!({"instance":self.instance,"epoch":epoch,"port":self.port.load(Ordering::SeqCst),
                    "ingress":if previous["instance"] == self.instance { previous["ingress"].clone() } else { serde_json::Value::Null }}),
            )
            .await?;
        Ok(epoch)
    }
}

pub async fn run(host: Arc<Host>, port: u16, remote: String, standby: bool) -> Result<()> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    host.port.store(u64::from(port), Ordering::SeqCst);
    let current = host
        .store
        .definition("host", "current")
        .await?
        .unwrap_or_default();
    if !standby || current["port"].as_u64() == Some(u64::from(port)) {
        host.activate_host().await?;
    }
    let gc = host.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(5));
        loop {
            tick.tick().await;
            if gc.retiring.load(Ordering::SeqCst) {
                break;
            }
            if let Err(error) = gc.collect_retired().await {
                tracing::error!(%error, "retired runtime collection failed");
            }
        }
    });
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let app = api::router(host.clone());
    let mut server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
    });
    let mut channels =
        std::collections::HashMap::<String, tokio::task::JoinHandle<Result<()>>>::new();
    let mut active_once = false;
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(100));
    loop {
        tokio::select! {
            _ = crate::shutdown_signal() => { host.retiring.store(true, Ordering::SeqCst); break; },
            result = &mut server => { result??; break; },
            _ = tick.tick() => {}
        }
        let current = host
            .store
            .definition("host", "current")
            .await?
            .unwrap_or_default();
        if current["instance"] != host.instance {
            if active_once && host.handoff_acknowledged(&current).await? {
                host.retiring.store(true, Ordering::SeqCst);
                break;
            }
            continue;
        }
        active_once = true;
        let servers = host.store.definitions("release_server").await?;
        let urls: Vec<String> = if servers.is_empty() {
            vec![remote.clone()]
        } else {
            servers
                .iter()
                .filter(|s| s["enabled"] == true)
                .filter_map(|s| s["url"].as_str().map(str::to_owned))
                .collect()
        };
        for url in urls {
            if channels.get(&url).is_some_and(|task| task.is_finished()) {
                channels.remove(&url);
            }
            channels.entry(url.clone()).or_insert_with(|| {
                let service: Arc<dyn NodeService> = host.clone();
                let token = host.token.clone();
                tokio::spawn(async move { opencoder_node::fleet::run(&url, &token, service).await })
            });
        }
    }
    host.retiring.store(true, Ordering::SeqCst);
    for (_, task) in channels {
        task.await??;
    }
    let _ = stop.send(());
    if !server.is_finished() {
        server.await??;
    }
    Ok(())
}
