//! Process-global registry of NAMED NFS exports — the generalization of
//! the former single `NFS_SLOT` in `api_agent_nfs`. Each export is a
//! `&'static str` key mapping to at most one live
//! [`NfsServerHandle`]; today the keys are [`AGENTS_EXPORT`] (the
//! agents root, `/api/agents/nfs`) and [`DAG_WASM_EXPORT`] (the DAG
//! wasm pool, `/api/dag/wasm/nfs`). Keeping the map process-global (the
//! same "static + tokio Mutex" pattern as `ACTIVATE_GATE`) leaves
//! `AppState` untouched: every construction site stays valid.
//!
//! Lifecycle invariants per key (same as the old single slot): start is
//! idempotent (a live handle is reused, never respawned — same bound
//! port); stop takes the handle out first so concurrent starts never
//! observe a half-shut server.

use std::collections::HashMap;

use opencoder_agents::{
    nfs_status, spawn_nfs_server, NfsServerHandle, NfsServerOpts, NfsServerStatus,
};

/// Key of the agents-root export (`/api/agents/nfs`).
pub const AGENTS_EXPORT: &str = "agents";
/// Key of the DAG wasm-pool export (`/api/dag/wasm/nfs`).
pub const DAG_WASM_EXPORT: &str = "dag-wasm";

/// Live NFS servers by export key. `spawn_nfs_server` runs the accept
/// loop on a dedicated OS thread (independent of any runtime), so the
/// map only owns handles; stop must go through
/// [`NfsServerHandle::shutdown`].
static EXPORTS: std::sync::LazyLock<tokio::sync::Mutex<HashMap<&'static str, NfsServerHandle>>> =
    std::sync::LazyLock::new(|| tokio::sync::Mutex::new(HashMap::new()));

/// Stopped snapshot for `key`: the [`nfs_status(None)`] shape, with the
/// port swapped for the export's own config default so a stopped
/// dag-wasm export never advertises the agents port (2049).
fn stopped(key: &'static str) -> NfsServerStatus {
    let mut status = nfs_status(None);
    status.port = match key {
        AGENTS_EXPORT => 2049,
        DAG_WASM_EXPORT => 2050,
        _ => 0,
    };
    status
}

/// Start (or reuse) the export under `key`: a live handle is returned as
/// `(status, false)` — never respawned — else the server is spawned from
/// `opts` under the map lock, inserted and returned as `(status, true)`.
/// `spawn_nfs_server` blocks briefly (bind + handshake with the server
/// thread) and its internals are sync, hence `spawn_blocking` while the
/// async lock is held (also what makes concurrent starts of the same key
/// serialize into one spawn).
pub async fn start(
    key: &'static str,
    opts: NfsServerOpts,
) -> Result<(NfsServerStatus, bool), String> {
    let mut exports = EXPORTS.lock().await;
    if let Some(handle) = exports.get(key) {
        return Ok((nfs_status(Some(handle)), false));
    }
    let spawned = tokio::task::spawn_blocking(move || spawn_nfs_server(&opts))
        .await
        .map_err(|e| format!("nfs spawn task: {e}"))?
        .map_err(|e| format!("nfs spawn: {e:#}"))?;
    let status = nfs_status(Some(&spawned));
    tracing::info!(
        export = key,
        host = %status.host,
        port = status.port,
        export_root = %status.export_root,
        "nfs export started"
    );
    exports.insert(key, spawned);
    Ok((status, true))
}

/// Stop and clear the export under `key` (idempotent). The handle is
/// taken out first so concurrent starts never observe a half-shut
/// server; `shutdown` parks up to its bounded timeout, hence
/// `spawn_blocking`. Returns `true` when a server was actually stopped.
pub async fn stop(key: &'static str) -> bool {
    let handle = EXPORTS.lock().await.remove(key);
    match handle {
        Some(handle) => {
            let _ = tokio::task::spawn_blocking(move || handle.shutdown()).await;
            tracing::info!(export = key, "nfs export stopped");
            true
        }
        None => false,
    }
}

/// Snapshot for `key`: `nfs_status` of the live handle, or the stopped
/// defaults. Sync on purpose (callers embed it in JSON handlers): a
/// `try_lock` miss means a start/stop is mid-flight on this key — the
/// stopped defaults are reported rather than blocking the caller.
pub fn status(key: &'static str) -> NfsServerStatus {
    match EXPORTS.try_lock() {
        Ok(exports) => exports
            .get(key)
            .map(|handle| nfs_status(Some(handle)))
            .unwrap_or_else(|| stopped(key)),
        Err(_) => stopped(key),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(root: &std::path::Path) -> NfsServerOpts {
        NfsServerOpts {
            export_root: root.to_path_buf(),
            host: "127.0.0.1".to_string(),
            port: 0, // ephemeral — 2049/2050 may be taken in CI
            read_only: true,
        }
    }

    /// One key runs the full lifecycle: first start spawns, the second
    /// reuses the live handle (started=false, same port), status reports
    /// running, stop parks it and reports stopped defaults again. The
    /// crate's other tests are sync `#[test]`s, so the runtime is built
    /// by hand (same pattern as `dag-wasm`'s scope tests).
    #[test]
    fn named_export_start_reuse_stop() {
        let dir = tempfile::tempdir().unwrap();
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let (first, started) = start("test-export", opts(dir.path())).await.unwrap();
                assert!(started, "first start must spawn");
                assert!(first.running);
                assert!(first.port > 0, "ephemeral port must resolve");

                let (again, started) = start("test-export", opts(dir.path())).await.unwrap();
                assert!(!started, "live handle must be reused, not respawned");
                assert_eq!(again.port, first.port, "reuse keeps the bound port");
                assert!(status("test-export").running);

                assert!(stop("test-export").await);
                let stopped = status("test-export");
                assert!(!stopped.running);
                assert_eq!(stopped.export_root, "");
                // Idempotent stop.
                assert!(!stop("test-export").await);
            });
    }
}
