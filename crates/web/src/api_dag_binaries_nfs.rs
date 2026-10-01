//! `/api/dag/binaries/nfs` — lifecycle control for the read-only NFS export
//! of the DAG binary pool, the second named export (`crate::nfs_exports`,
//! key [`DAG_BINARY_EXPORT`]; the agents root was the first). GET reports
//! the live snapshot plus the resolved pool root; POST `{enabled}`
//! starts/stops the server explicitly.
//!
//! [`configured_dag_binary`] is the scope middleware every `/api/dag/binaries*`
//! request passes through (inside the bearer layer): it resolves the
//! pool root from the workdir and injects it into the task-local scope,
//! because the pool — unlike the agents root — has no home-dir fallback
//! (`opencoder_dag_binary::binary_root()` bottoms out at `None`). An
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

use opencoder_dag_binary::scope;

use crate::nfs_exports::{self, DAG_BINARY_EXPORT};
use crate::AppState;

/// Per-workdir data-dir default: `<data_dir>/dag/binary` (never created
/// here; the write path `create_dir_all`s pool dirs under it).
fn data_dir_default(workdir: &Path) -> PathBuf {
    opencoder_core::data_dir_for(workdir)
        .join("dag")
        .join("binary")
}

/// The pool root for `workdir`: (a) an already-scoped/overridden root
/// (`opencoder_dag_binary::binary_root` — tests pin the process-global
/// override; the middleware's own scope would also be visible), else
/// (b) config `dag.binary_dir`, else (c) the data-dir default.
fn resolve_root(workdir: &Path) -> PathBuf {
    if let Some(root) = opencoder_dag_binary::binary_root() {
        return root;
    }
    if let Ok(config) = Config::load(workdir) {
        if let Some(dir) = config.dag.binary_dir {
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

/// Scope middleware: `/api/dag/binaries` and everything under
/// `/api/dag/binaries/` run with the resolved pool root in the task-local
/// scope; all other requests pass through untouched. A root is ALWAYS
/// injected (no home-dir fallback to defer to).
pub async fn configured_dag_binary(
    State(state): State<Arc<AppState>>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let path = request.uri().path();
    if path != "/api/dag/binaries" && !path.starts_with("/api/dag/binaries/") {
        return next.run(request).await;
    }
    let root = resolve_root(&state.workdir);
    scope::with_root(Some(root), next.run(request)).await
}

/// GET /api/dag/binaries/nfs — live snapshot of the dag-binary export plus the
/// resolved pool root it serves (stopped defaults when not running).
pub async fn nfs_get(State(state): State<Arc<AppState>>) -> Response {
    Json(json!({
        "ok": true,
        "root": resolve_root(&state.workdir).display().to_string(),
        "status": status_value(&nfs_exports::status(DAG_BINARY_EXPORT).await),
    }))
    .into_response()
}

#[derive(Deserialize)]
pub struct SetBody {
    pub enabled: bool,
}

/// POST /api/dag/binaries/nfs — explicit lifecycle switch (same contract as
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
        if !config.dag.nfs.read_only {
            return error_500("DAG binary exports must be read-only".into());
        }
        let opts = NfsServerOpts {
            export_root: resolve_root(&state.workdir),
            host: config.dag.nfs.host.clone(),
            port: config.dag.nfs.port,
            read_only: config.dag.nfs.read_only,
        };
        match nfs_exports::start(DAG_BINARY_EXPORT, opts).await {
            Ok((status, started)) => {
                Json(json!({ "ok": true, "status": status_value(&status), "started": started }))
                    .into_response()
            }
            Err(e) => error_500(e),
        }
    } else {
        nfs_exports::stop(DAG_BINARY_EXPORT).await;
        Json(json!({
            "ok": true,
            "status": status_value(&nfs_exports::status(DAG_BINARY_EXPORT).await),
            "started": false,
        }))
        .into_response()
    }
}

/// Start the configured immutable binary export before accepting requests.
pub async fn autostart(workdir: &Path) -> anyhow::Result<()> {
    let config = Config::load(workdir)?;
    if !config.dag.nfs.enabled {
        return Ok(());
    }
    anyhow::ensure!(
        config.dag.nfs.read_only,
        "DAG binary exports must be read-only"
    );
    let opts = NfsServerOpts {
        export_root: resolve_root(workdir),
        host: config.dag.nfs.host.clone(),
        port: config.dag.nfs.port,
        read_only: config.dag.nfs.read_only,
    };
    nfs_exports::start(DAG_BINARY_EXPORT, opts)
        .await
        .map_err(anyhow::Error::msg)?;
    Ok(())
}
