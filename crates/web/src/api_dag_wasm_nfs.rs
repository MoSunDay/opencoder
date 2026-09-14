//! `/api/dag/wasm/nfs` — lifecycle control for the read-only NFS export
//! of the DAG wasm pool, the second named export (`crate::nfs_exports`,
//! key [`DAG_WASM_EXPORT`]; the agents root was the first). GET reports
//! the live snapshot plus the resolved pool root; POST `{enabled}`
//! starts/stops the server explicitly.
//!
//! [`configured_dag_wasm`] is the scope middleware every `/api/dag/wasm*`
//! request passes through (inside the bearer layer): it resolves the
//! pool root from the workdir and injects it into the task-local scope,
//! because the pool — unlike the agents root — has no home-dir fallback
//! (`opencoder_dag_wasm::wasm_root()` bottoms out at `None`). An
//! already-scoped/overridden root (tests, embedders) always wins.
//!
//! This file is `#[path]`-shared into control, whose `AppState` also
//! carries `workdir: PathBuf` — state usage stays `state.workdir` only.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::State;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use opencoder_agents::{NfsServerOpts, NfsServerStatus};
use opencoder_core::Config;
use serde::Deserialize;
use serde_json::{json, Value};

use opencoder_dag_wasm::scope;

use crate::nfs_exports::{self, DAG_WASM_EXPORT};
use crate::AppState;

/// Per-workdir data-dir default: `<data_dir>/dag/wasm` (never created
/// here; the write path `create_dir_all`s pool dirs under it).
fn data_dir_default(workdir: &Path) -> PathBuf {
    opencoder_core::data_dir_for(workdir)
        .join("dag")
        .join("wasm")
}

/// The pool root for `workdir`: (a) an already-scoped/overridden root
/// (`opencoder_dag_wasm::wasm_root` — tests pin the process-global
/// override; the middleware's own scope would also be visible), else
/// (b) config `dag.wasm_dir`, else (c) the data-dir default.
fn resolve_root(workdir: &Path) -> PathBuf {
    if let Some(root) = opencoder_dag_wasm::wasm_root() {
        return root;
    }
    if let Ok(config) = Config::load(workdir) {
        if let Some(dir) = config.dag.wasm_dir {
            return dir;
        }
    }
    data_dir_default(workdir)
}

fn error_500(msg: String) -> Response {
    (
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "ok": false, "error": msg })),
    )
        .into_response()
}

fn status_value(status: &NfsServerStatus) -> Value {
    serde_json::to_value(status).unwrap_or_else(|_| json!({}))
}

/// Scope middleware: `/api/dag/wasm` and everything under
/// `/api/dag/wasm/` run with the resolved pool root in the task-local
/// scope; all other requests pass through untouched. A root is ALWAYS
/// injected (no home-dir fallback to defer to).
pub async fn configured_dag_wasm(
    State(state): State<Arc<AppState>>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let path = request.uri().path();
    if path != "/api/dag/wasm" && !path.starts_with("/api/dag/wasm/") {
        return next.run(request).await;
    }
    let root = resolve_root(&state.workdir);
    scope::with_root(Some(root), next.run(request)).await
}

/// GET /api/dag/wasm/nfs — live snapshot of the dag-wasm export plus the
/// resolved pool root it serves (stopped defaults when not running).
pub async fn nfs_get(State(state): State<Arc<AppState>>) -> Response {
    Json(json!({
        "ok": true,
        "root": resolve_root(&state.workdir).display().to_string(),
        "status": status_value(&nfs_exports::status(DAG_WASM_EXPORT)),
    }))
    .into_response()
}

#[derive(Deserialize)]
pub struct SetBody {
    pub enabled: bool,
}

/// POST /api/dag/wasm/nfs — explicit lifecycle switch (same contract as
/// the agents export): `enabled:true` is idempotent (reuse, not
/// respawn); `enabled:false` stops and clears, also idempotent. Host,
/// port and read-only come from config `dag.nfs`, the export root is
/// [`resolve_root`] — so config edits apply on the next start.
pub async fn nfs_post(State(state): State<Arc<AppState>>, Json(body): Json<SetBody>) -> Response {
    if body.enabled {
        let config = match Config::load(&state.workdir) {
            Ok(c) => c,
            Err(e) => return error_500(format!("config: {e:#}")),
        };
        let opts = NfsServerOpts {
            export_root: resolve_root(&state.workdir),
            host: config.dag.nfs.host.clone(),
            port: config.dag.nfs.port,
            read_only: config.dag.nfs.read_only,
        };
        match nfs_exports::start(DAG_WASM_EXPORT, opts).await {
            Ok((status, started)) => {
                Json(json!({ "ok": true, "status": status_value(&status), "started": started }))
                    .into_response()
            }
            Err(e) => error_500(e),
        }
    } else {
        nfs_exports::stop(DAG_WASM_EXPORT).await;
        Json(json!({
            "ok": true,
            "status": status_value(&nfs_exports::status(DAG_WASM_EXPORT)),
            "started": false,
        }))
        .into_response()
    }
}

/// Daemon autostart seam, called from `serve` before the HTTP listener
/// binds: when `dag.nfs.enabled` is set, bring the export up so it is
/// live by the time the API answers. Failure is logged and swallowed —
/// a broken export must never take the daemon down (`GET
/// /api/dag/wasm/nfs` will simply report stopped).
pub async fn autostart(workdir: &Path) {
    let config = match Config::load(workdir) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("dag wasm nfs autostart skipped, config load failed: {e:#}");
            return;
        }
    };
    if !config.dag.nfs.enabled {
        return;
    }
    let opts = NfsServerOpts {
        export_root: resolve_root(workdir),
        host: config.dag.nfs.host.clone(),
        port: config.dag.nfs.port,
        read_only: config.dag.nfs.read_only,
    };
    if let Err(e) = nfs_exports::start(DAG_WASM_EXPORT, opts).await {
        tracing::warn!("dag wasm nfs autostart failed (continuing): {e}");
    }
}
