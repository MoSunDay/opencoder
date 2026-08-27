//! Node runtime: the main loop that keeps this machine registered to a
//! central server, heartbeats its liveness, claims dispatched tasks, and runs
//! them one at a time.
//!
//! Loop shape (single [`tokio::select!`], every arm non-blocking):
//! - `heartbeat` tick — liveness touch while idle.
//! - `claim` tick — idle-only FIFO claim poll. A claimed task is executed
//!   serially inside the arm, so no second task can start until it returns;
//!   during that window a dedicated per-task heartbeater keeps liveness fresh
//!   AND feeds the executor's cancel flag from `cancel_task_ids`.
//! - shutdown — armed by Ctrl-C/SIGTERM; while a task is active it converges
//!   through the SAME cancel flag as a server-side stop, so exactly one
//!   reporting protocol exists (`status=cancelled`).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use opencoder_llm::{ChatClient, ChatStream};
use opencoder_store::{LibsqlStore, Store};
use tokio::sync::watch;
use tracing::{info, warn};

use opencoder_core::node_protocol::ClaimedTask;

use crate::control::{handle_control, Inflight};
use crate::executor::{execute, ExecDeps};
use crate::uplink::Uplink;

/// Liveness tick sent to the server. Short enough that a lost node crosses
/// into `lost` within a few intervals; long enough to stay cheap.
pub const DEFAULT_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);

/// Non-blocking claim poll cadence while idle.
pub const DEFAULT_CLAIM_INTERVAL: Duration = Duration::from_millis(1500);

/// Startup registration retry budget (network errors / transient rejections).
pub const REGISTER_ATTEMPTS: usize = 5;

#[derive(Clone)]
pub struct NodeOpts {
    /// Friendly unique node name; re-registering replaces the old row.
    pub name: String,
    pub remote: String,
    /// Resolved bearer token (the CLI already applied the client-token rules:
    /// flag or OPENCODER_SERVER_TOKEN env, NEVER auto-generated).
    pub token: String,
    pub workdir: PathBuf,
    pub heartbeat_interval: Duration,
    pub claim_interval: Duration,
    pub version: String,
    /// Local DB directory override; defaults to the serve() data-dir rule
    /// ([`opencoder_core::data_dir_for`] on workdir) so a server and a node
    /// sharing one machine share one store.
    pub local_store_dir: Option<PathBuf>,
}

impl NodeOpts {
    /// Fast-fail usage errors before any network traffic (mirrors the
    /// client-side contract: no token means refuse, never invent one).
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            bail!("node name must not be empty");
        }
        if self.remote.trim().is_empty() {
            bail!("remote base URL must not be empty");
        }
        if self.token.trim().is_empty() {
            bail!(
                "missing bearer token: pass --token or set OPENCODER_SERVER_TOKEN \
                 (a node never auto-generates tokens)"
            );
        }
        Ok(())
    }
}

/// Entry point wired from the CLI: run this machine as an execution node.
/// Returns when a graceful-shutdown signal fires or an unrecoverable startup
/// error occurs (bad options, unreachable server past the retry budget, bad
/// local store/config).
pub async fn run_node(opts: NodeOpts, override_client: Option<Arc<dyn ChatStream>>) -> Result<()> {
    opts.validate()?;
    let uplink = Uplink::new(&opts.remote, &opts.token)?;
    let node_id = register_with_retry(&uplink, &opts).await?;

    let store_dir = opts
        .local_store_dir
        .clone()
        .unwrap_or_else(|| opencoder_core::data_dir_for(&opts.workdir));
    tokio::fs::create_dir_all(&store_dir).await.ok();
    let store: Arc<dyn Store> = Arc::new(
        LibsqlStore::open(store_dir.join("opencoder.db"))
            .await
            .context("open local node store")?,
    );

    let client: Arc<dyn ChatStream> = match override_client {
        Some(c) => c,
        None => build_default_client(&opts)?,
    };

    info!(
        name = %opts.name,
        %node_id,
        remote = %opts.remote,
        workdir = %opts.workdir.display(),
        "node online"
    );

    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    spawn_shutdown_signal(shutdown_tx);

    // Shared control-task dedup: the same control can be delivered twice
    // within milliseconds (claim reply racing a heartbeat batch).
    let inflight = Inflight::new();

    let mut hb_tick = tokio::time::interval(opts.heartbeat_interval);
    let mut claim_tick = tokio::time::interval(opts.claim_interval);
    hb_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    claim_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        // A fresh future per iteration keeps the receiver reusable and lets
        // `biased` prioritize graceful exit over claiming new work.
        tokio::select! {
            biased;
            _ = crate::await_flag(&mut shutdown_rx) => {
                info!("shutdown signal received; node exiting");
                return Ok(());
            }
            _ = hb_tick.tick() => {
                match uplink.heartbeat(&node_id).await {
                    Ok(resp) => {
                        // Idle nodes normally see nothing here; controls can
                        // stack up while the claim arm was busy executing.
                        for task in &resp.controls {
                            handle_control(&uplink, &store, &inflight, &node_id, task).await;
                        }
                    }
                    Err(e) => warn!(error = %e, "heartbeat failed (retrying next tick)"),
                }
            }
            _ = claim_tick.tick() => {
                match uplink.claim_next(&node_id).await {
                    Ok(resp) => {
                        // Durable work is preferred by the server; a control
                        // task rides along only when no task was due.
                        if let Some(task) = &resp.control {
                            handle_control(&uplink, &store, &inflight, &node_id, task).await;
                        }
                        if let Some(task) = resp.task {
                            run_task(
                                &uplink,
                                &store,
                                &client,
                                &opts,
                                &task,
                                &node_id,
                                &shutdown_rx,
                                &inflight,
                            )
                            .await;
                        }
                    }
                    Err(e) => warn!(error = %e, "claim poll failed"),
                }
            }
        }
    }
}

/// Execute ONE task serially under its own heartbeater.
///
/// Cancel inputs converge onto one watched flag fed from two sources: a
/// heartbeat whose `cancel_task_ids` contains this task, or process shutdown.
/// The executor races that flag against the drain so an aborted run still
/// closes through the session's own interrupt path and reports `cancelled`.
#[allow(clippy::too_many_arguments)]
async fn run_task(
    uplink: &Uplink,
    store: &Arc<dyn Store>,
    client: &Arc<dyn ChatStream>,
    opts: &NodeOpts,
    task: &ClaimedTask,
    node_id: &str,
    shutdown_rx: &watch::Receiver<bool>,
    inflight: &Inflight,
) {
    info!(
        task_id = %task.task_id,
        session_id = %task.session_id,
        prompt_chars = task.prompt.chars().count(),
        "claimed node task"
    );

    let (cancel_tx, cancel_rx) = watch::channel(false);
    let hb = spawn_heartbeater(
        uplink.clone(),
        Arc::clone(store),
        inflight.clone(),
        node_id.to_string(),
        opts.heartbeat_interval,
        task.task_id.clone(),
        cancel_tx.clone(),
    );

    // Shutdown converges through the SAME cancel flag as a server-side stop,
    // so exactly one reporting protocol exists (executor reports cancelled).
    let fwd_tx = cancel_tx.clone();
    let mut fwd_shutdown = shutdown_rx.clone();
    tokio::spawn(async move {
        crate::await_flag(&mut fwd_shutdown).await;
        info!("shutdown observed during task; requesting local cancellation");
        let _ = fwd_tx.send(true);
    });

    let config = match opencoder_core::Config::load(&opts.workdir) {
        Ok(c) => c,
        Err(e) => {
            warn!(task_id = %task.task_id, error = %e, "config load failed; failing task");
            best_effort_status(
                uplink,
                &task.task_id,
                "error",
                Some(format!("config load: {e:#}")),
            )
            .await;
            hb.abort();
            return;
        }
    };
    let deps = ExecDeps {
        store: Arc::clone(store),
        client: Arc::clone(client),
        workdir: opts.workdir.clone(),
        config,
    };
    match execute(uplink, deps, task, cancel_rx).await {
        Ok(()) => info!(task_id = %task.task_id, "node task finished"),
        Err(e) => warn!(task_id = %task.task_id, error = %e, "node task ended with error"),
    }
    // The heartbeater parks once the executor reported a terminal state;
    // abort only cleans up that parked waiter.
    hb.abort();
}

/// Per-task heartbeat duty loop. Exits as soon as cancellation for THIS task
/// is observed (liveness afterwards is irrelevant: execution is collapsing),
/// leaving the outer idle loop to resume its own ticks after `execute`.
fn spawn_heartbeater(
    uplink: Uplink,
    store: Arc<dyn Store>,
    inflight: Inflight,
    node_id: String,
    interval: Duration,
    task_id: String,
    cancel_tx: watch::Sender<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(interval);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            match uplink.heartbeat(&node_id).await {
                Ok(resp) => {
                    if resp.cancel_task_ids.iter().any(|id| id == &task_id) {
                        info!(%task_id, "cancellation observed on heartbeat");
                        let _ = cancel_tx.send(true);
                        return;
                    }
                    // A BUSY worker never polls claim: the heartbeat is its
                    // only guaranteed control-task delivery channel.
                    for task in &resp.controls {
                        handle_control(&uplink, &store, &inflight, &node_id, task).await;
                    }
                }
                Err(e) => warn!(%task_id, error = %e, "busy heartbeat failed"),
            }
        }
    })
}

/// Fire-and-forget terminal report used before execution even starts (config
/// load failure). Failures only log: the server's task stuck in `running`
/// converges via its own cancellation path.
async fn best_effort_status(uplink: &Uplink, tid: &str, status: &str, error: Option<String>) {
    if let Err(e) = uplink.report_status(tid, status, error).await {
        warn!(task_id = %tid, status, error = %e, "status report failed");
    }
}

/// Register with a bounded retry budget; failures here are fatal because a
/// node cannot receive work without identity. Backoff grows linearly.
async fn register_with_retry(uplink: &Uplink, opts: &NodeOpts) -> Result<String> {
    let mut last = None;
    for attempt in 1..=REGISTER_ATTEMPTS {
        match uplink
            .register(&opts.name, &opts.version, opts.workdir.to_str())
            .await
        {
            Ok(resp) => return Ok(resp.node_id),
            Err(e) => {
                warn!(attempt, error = %e, "registration failed; retrying");
                last = Some(e);
                tokio::time::sleep(Duration::from_millis(500 * attempt as u64)).await;
            }
        }
    }
    Err(last.unwrap_or_else(|| anyhow!("registration failed"))).context("register node")
}

/// Real LLM backend from the resolved config (same construction as
/// `cli/src/run.rs`); used whenever the caller did not inject a test double.
fn build_default_client(opts: &NodeOpts) -> Result<Arc<dyn ChatStream>> {
    let config = opencoder_core::Config::load(&opts.workdir)?;
    let ep = config.resolve_endpoint()?;
    let client = ChatClient::new_with_read_timeout(
        &ep.base_url,
        &ep.api_key,
        &ep.headers,
        config.stream_idle_timeout(),
        config.network.proxy.as_deref(),
    )?;
    Ok(Arc::new(client))
}

/// Arm graceful shutdown on Ctrl-C / SIGTERM (first signal requests a clean
/// stop; the OS-level second-signal force-exit stays available to operators).
fn spawn_shutdown_signal(tx: watch::Sender<bool>) {
    tokio::spawn(async move {
        let ctrl_c = async {
            let _ = tokio::signal::ctrl_c().await;
        };
        #[cfg(unix)]
        let terminate = async {
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(mut s) => {
                    s.recv().await;
                }
                Err(_) => std::future::pending::<()>().await,
            }
        };
        #[cfg(not(unix))]
        let terminate = std::future::pending::<()>();

        tokio::select! {
            () = ctrl_c => {}
            () = terminate => {}
        }
        info!("shutdown requested; finishing current step");
        let _ = tx.send(true);
    });
}
