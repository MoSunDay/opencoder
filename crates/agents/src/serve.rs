//! NFSv3 server plumbing for the agents root.
//!
//! `nfsserve` multiplexes NFS (100003), MOUNT (100005) and a fake portmap
//! (100000) on a **single TCP listener**, so no external mountd/rpcbind is
//! needed — clients just have to aim both `port=` and `mountport=` at it
//! (see [`spawn_nfs_server`] docs for the mount incantation).
//!
//! The export is read-only in this first version: [`crate::nfs`] rejects
//! every mutating op with `NFS3ERR_ROFS` even when `read_only == false` in
//! the options (write support is future work); the flag still flows
//! through status/config so later waves can flip it.

use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use nfsserve::tcp::{NFSTcp as _, NFSTcpListener};
use serde::Serialize;

use crate::nfs::{agents_fs, ReadOnlyAgentsFs};

/// How long [`NfsServerHandle::shutdown`] waits for the accept loop to die.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// Options for [`spawn_nfs_server`]. `port: 0` binds an ephemeral port
/// (resolve with [`NfsServerHandle::local_addr`]).
pub struct NfsServerOpts {
    pub export_root: PathBuf,
    pub host: String,
    pub port: u16,
    pub read_only: bool,
}

/// Snapshot consumed by the web layer. Serde shape is part of the public
/// contract — field names and casing must not change.
#[derive(Debug, Clone, Serialize)]
pub struct NfsServerStatus {
    pub running: bool,
    pub host: String,
    pub port: u16,
    pub read_only: bool,
    pub export_root: String,
}

/// Running server: the accept loop lives on a dedicated runtime thread
/// (one OS thread per server; it parks in `accept` between requests).
/// Dropping without [`shutdown`](Self::shutdown) leaks that thread and the
/// port until process exit — call `shutdown` for a graceful stop.
pub struct NfsServerHandle {
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    done: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    addr: SocketAddr,
    export_root: PathBuf,
    read_only: bool,
}

impl NfsServerHandle {
    /// The address the listener actually bound (`port: 0` ⇒ ephemeral).
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        Ok(self.addr)
    }

    /// The exported directory (canonicalized at spawn time).
    pub fn export_root(&self) -> &Path {
        &self.export_root
    }

    /// Whether the server advertises itself read-only. Always effective in
    /// this version — see the module docs.
    pub fn read_only(&self) -> bool {
        self.read_only
    }

    /// Graceful stop: signals the accept loop, waits for it to finish
    /// (bounded by `SHUTDOWN_TIMEOUT`), never panics. The port is released
    /// as soon as the loop's listener is dropped — guaranteed before the
    /// `done` flag observed here flips.
    pub fn shutdown(mut self) {
        // `changed()` also resolves when the sender is dropped, so even a
        // crashed server thread stops being waited on here.
        let _ = self.shutdown_tx.send(true);
        let deadline = Instant::now() + SHUTDOWN_TIMEOUT;
        while !self.done.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        // Join only when the loop reported completion — joining a stuck
        // thread would trade "bounded" for "hang".
        if self.done.load(Ordering::SeqCst) {
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }
}

/// Bind + spawn the NFSv3 server.
///
/// The accept loop runs on a **dedicated runtime thread** (its own
/// single-threaded tokio runtime), so this works from any thread — with or
/// without an ambient runtime — and stopping it never interferes with the
/// caller's executor. `port: 0` ⇒ ephemeral. `read_only` is recorded (and
/// reported by [`nfs_status`]) but the export is read-only regardless in
/// this version.
///
/// Mount from a Linux client (nfsserve's portmap answers on the same port,
/// so point both protocols at it):
///
/// ```text
/// mount -t nfs -o vers=3,port=2049,mountport=2049,nolock 127.0.0.1:/ /mnt/agents
/// ```
pub fn spawn_nfs_server(opts: &NfsServerOpts) -> anyhow::Result<NfsServerHandle> {
    let meta = std::fs::metadata(&opts.export_root)
        .with_context(|| format!("nfs export_root {:?} is not accessible", opts.export_root))?;
    if !meta.is_dir() {
        anyhow::bail!("nfs export_root {:?} is not a directory", opts.export_root);
    }
    let root = opts
        .export_root
        .canonicalize()
        .context("canonicalize nfs export_root")?;
    spawn_listener(root, opts)
}

/// Bind + run the listener on a dedicated thread owning a single-threaded
/// runtime. `select!` between the shutdown watch and nfsserve's accept loop
/// is the cancellation path: dropping the listener there releases the port
/// before the `done` flag flips.
fn spawn_listener(root: PathBuf, opts: &NfsServerOpts) -> anyhow::Result<NfsServerHandle> {
    let fs: ReadOnlyAgentsFs = agents_fs(root.clone());
    let bind = format!("{}:{}", opts.host, opts.port);
    let (tx, rx) = std::sync::mpsc::channel::<anyhow::Result<SocketAddr>>();
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);
    let done = Arc::new(AtomicBool::new(false));
    let done_flag = done.clone();
    let thread = std::thread::Builder::new()
        .name("opencoder-nfs".to_string())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = tx.send(Err(anyhow::Error::new(e).context("build nfs runtime")));
                    return;
                }
            };
            rt.block_on(async move {
                let listener = match NFSTcpListener::bind(&bind, fs).await {
                    Ok(l) => l,
                    Err(e) => {
                        let _ = tx
                            .send(Err(anyhow::Error::new(e)
                                .context(format!("bind nfs listener on {bind}"))));
                        return;
                    }
                };
                // Export name stays "/" so any mount path under it resolves
                // (MOUNT strips the export prefix and walks path_to_id).
                let addr = SocketAddr::new(listener.get_listen_ip(), listener.get_listen_port());
                let _ = tx.send(Ok(addr));
                tokio::select! {
                    biased;
                    _ = shutdown_rx.changed() => {}
                    res = listener.handle_forever() => {
                        if let Err(e) = res {
                            tracing::warn!("nfs accept loop terminated: {e}");
                        }
                    }
                }
                done_flag.store(true, Ordering::SeqCst);
            });
        })
        .context("spawn nfs server thread")?;
    let addr = rx
        .recv_timeout(SHUTDOWN_TIMEOUT)
        .context("nfs listener did not come up in time")??;
    Ok(NfsServerHandle {
        shutdown_tx,
        done,
        thread: Some(thread),
        addr,
        export_root: root,
        read_only: opts.read_only,
    })
}

/// Status snapshot; `None` ⇒ the documented "stopped" defaults.
pub fn nfs_status(h: Option<&NfsServerHandle>) -> NfsServerStatus {
    match h {
        None => NfsServerStatus {
            running: false,
            host: "127.0.0.1".to_string(),
            port: 2049,
            read_only: true,
            export_root: String::new(),
        },
        Some(h) => NfsServerStatus {
            running: true,
            host: h.addr.ip().to_string(),
            port: h.addr.port(),
            read_only: h.read_only,
            export_root: h.export_root.display().to_string(),
        },
    }
}

/// Options from the `agent.nfs` config block plus the agents root. The
/// `enabled` flag is *not* consumed here — callers (daemon autostart) gate
/// on it themselves. A missing root (no override, no env var, no home)
/// yields an empty path, which only errors later, at spawn.
pub fn default_opts_from_config(cfg: &opencoder_core::config::Config) -> NfsServerOpts {
    let nfs = &cfg.agent.nfs;
    let export_root = cfg
        .agent
        .agents_dir
        .clone()
        .or_else(opencoder_core::agent::agents_dir)
        .unwrap_or_default();
    NfsServerOpts {
        export_root,
        host: nfs.host.clone(),
        port: nfs.port,
        read_only: nfs.read_only,
    }
}

#[cfg(test)]
mod tests;
