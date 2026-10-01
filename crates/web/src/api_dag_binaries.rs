//! `/api/dag/binaries` — CRUD over the versioned binary pool
//! (`opencoder_dag_binary`), mirroring the agents resource-pool HTTP
//! surface ([`crate::api_agent_resources`]): base64-uploaded binaries,
//! monotonic never-reused version numbers, pointer-only rollback,
//! atomic writes (temp sibling + rename). The pool root is NOT managed
//! here — it is injected by the [`crate::api_dag_binaries_nfs`] scope
//! middleware (config `dag.binary_dir` or the per-workdir data-dir
//! default), or pinned by the process-global override (tests,
//! embedders). Handlers are root-free otherwise: they take
//! `State<Arc<AppState>>` purely for symmetry, so control can reuse the
//! file via `#[path]`.

use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde::Deserialize;
use serde_json::{json, Value};

use opencoder_dag_binary::{
    binary_bin, binary_root, create_binary, delete_binary, list_pools, read_pool_meta,
    read_version_meta, rollback_binary, save_binary_version, validate_binary_bytes, validate_name,
    BinaryVersionMeta, MAX_BINARY_BYTES,
};

use crate::AppState;

fn error_400(msg: String) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "ok": false, "error": msg })),
    )
        .into_response()
}

fn error_404(msg: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "ok": false, "error": msg })),
    )
        .into_response()
}

fn error_500(msg: String) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "ok": false, "error": msg })),
    )
        .into_response()
}

fn error_409(msg: String) -> Response {
    (
        StatusCode::CONFLICT,
        Json(json!({ "ok": false, "error": msg })),
    )
        .into_response()
}

/// Map write-path io errors onto REST statuses (envs idiom): `NotFound` ⇒
/// 404, `AlreadyExists` ⇒ 409, `InvalidInput`/`InvalidData` ⇒ 400.
fn io_error_response(ctx: &str, e: io::Error) -> Response {
    match e.kind() {
        io::ErrorKind::NotFound => error_404(&format!("{ctx}: {e}")),
        io::ErrorKind::AlreadyExists => error_409(format!("{ctx}: {e}")),
        io::ErrorKind::InvalidInput | io::ErrorKind::InvalidData => {
            error_400(format!("{ctx}: {e}"))
        }
        _ => error_500(format!("{ctx}: {e}")),
    }
}

/// The binary-pool root for this request (request scope → embedding override).
/// `None` when nothing injected one — the pool has no home-dir fallback,
/// so an unconfigured daemon must fail loudly (500), not write into
/// nowhere.
fn pool_root() -> Option<PathBuf> {
    binary_root()
}

fn version_value(meta: &BinaryVersionMeta) -> Value {
    serde_json::to_value(meta).unwrap_or_else(|_| json!({}))
}

/// Decode + cap-check the base64 binary. Every failure is a 400 message.
fn decode_binary(binary_b64: &str) -> Result<Vec<u8>, String> {
    let bytes = B64
        .decode(binary_b64.as_bytes())
        .map_err(|e| format!("bad base64 in `binary_b64`: {e}"))?;
    if bytes.len() > MAX_BINARY_BYTES {
        return Err(format!(
            "binary is {} bytes, over the 32 MiB cap",
            bytes.len()
        ));
    }
    validate_binary_bytes(&bytes)?;
    Ok(bytes)
}

#[derive(Deserialize)]
pub struct CreateBody {
    name: String,
    #[serde(default)]
    description: String,
    binary_b64: String,
}

/// POST /api/dag/binaries — create a pool entry at v1. Duplicate names are a
/// 409 (the write path itself would happily version on top; the
/// create/put split is an HTTP-layer contract, same as the agents
/// pools).
pub async fn create(State(_state): State<Arc<AppState>>, Json(body): Json<CreateBody>) -> Response {
    let name = body.name.trim().to_string();
    if let Err(e) = validate_name(&name) {
        return error_400(format!("invalid binary pool name: {e}"));
    }
    let bytes = match decode_binary(&body.binary_b64) {
        Ok(bytes) => bytes,
        Err(e) => return error_400(e),
    };
    let Some(root) = pool_root() else {
        return error_500("dag binary pool root not configured".to_string());
    };
    if read_pool_meta(&root, &name).is_some() {
        return error_409(format!("binary pool entry already exists: {name}"));
    }
    match create_binary(&root, &name, &body.description, &bytes) {
        Ok(version) => (
            StatusCode::CREATED,
            Json(json!({ "ok": true, "name": name, "version": version })),
        )
            .into_response(),
        Err(e) => io_error_response("save binary version", e),
    }
}

#[derive(Deserialize)]
pub struct PutBody {
    description: String,
    binary_b64: String,
}

/// PUT /api/dag/binaries/:name — append a new version (monotonic, never
/// reusing numbers, even across rollbacks) and make it current. 404 when
/// the pool does not exist: PUT never creates (POST does).
pub async fn put_version(
    State(_state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<PutBody>,
) -> Response {
    let Some(root) = pool_root() else {
        return error_500("dag binary pool root not configured".to_string());
    };
    if read_pool_meta(&root, &name).is_none() {
        return error_404(&format!("unknown binary pool: {name}"));
    }
    let bytes = match decode_binary(&body.binary_b64) {
        Ok(bytes) => bytes,
        Err(e) => return error_400(e),
    };
    match save_binary_version(&root, &name, &body.description, &bytes) {
        Ok(version) => {
            Json(json!({ "ok": true, "name": name, "version": version })).into_response()
        }
        Err(e) => io_error_response("save binary version", e),
    }
}

/// GET /api/dag/binaries — every pool with its current version meta (null
/// when the current pointer predates a missing version dir).
pub async fn list(State(_state): State<Arc<AppState>>) -> Response {
    let Some(root) = pool_root() else {
        return error_500("dag binary pool root not configured".to_string());
    };
    let pools: Vec<Value> = list_pools(&root)
        .into_iter()
        .filter_map(|name| {
            let meta = read_pool_meta(&root, &name)?;
            let current_version = read_version_meta(&root, &name, meta.current)
                .map(|m| version_value(&m))
                .unwrap_or(Value::Null);
            Some(json!({
                "name": meta.name,
                "description": meta.description,
                "current": meta.current,
                "updated_at": meta.updated_at,
                "current_version": current_version,
            }))
        })
        .collect();
    Json(json!({ "ok": true, "pools": pools })).into_response()
}

/// GET /api/dag/binaries/:name — pool meta plus the full version history
/// (version metas, null-tolerant for pruned/legacy entries).
pub async fn get(State(_state): State<Arc<AppState>>, Path(name): Path<String>) -> Response {
    let Some(root) = pool_root() else {
        return error_500("dag binary pool root not configured".to_string());
    };
    let Some(meta) = read_pool_meta(&root, &name) else {
        return error_404(&format!("unknown binary pool: {name}"));
    };
    let history: Vec<Value> = meta
        .history
        .iter()
        .map(|v| {
            read_version_meta(&root, &name, *v)
                .map(|m| version_value(&m))
                .unwrap_or(Value::Null)
        })
        .collect();
    Json(json!({
        "name": meta.name,
        "description": meta.description,
        "created_at": meta.created_at,
        "updated_at": meta.updated_at,
        "current": meta.current,
        "history": history,
    }))
    .into_response()
}

/// DELETE /api/dag/binaries/:name — the whole `<name>/` tree (the only
/// operation that removes version dirs). 404 first so a missing pool is
/// never silently "deleted".
pub async fn delete(State(_state): State<Arc<AppState>>, Path(name): Path<String>) -> Response {
    let Some(root) = pool_root() else {
        return error_500("dag binary pool root not configured".to_string());
    };
    if read_pool_meta(&root, &name).is_none() {
        return error_404(&format!("unknown binary pool: {name}"));
    }
    match delete_binary(&root, &name) {
        Ok(()) => Json(json!({ "ok": true, "deleted": name })).into_response(),
        Err(e) => io_error_response("delete binary pool", e),
    }
}

#[derive(Deserialize)]
pub struct RollbackBody {
    version: u32,
}

/// POST /api/dag/binaries/:name/rollback — pointer-only switch of `current`
/// back to a version still in the pool's history; version dirs are never
/// deleted by it, so a later save keeps numbering monotonic.
pub async fn rollback(
    State(_state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<RollbackBody>,
) -> Response {
    let Some(root) = pool_root() else {
        return error_500("dag binary pool root not configured".to_string());
    };
    match rollback_binary(&root, &name, body.version) {
        Ok(()) => {
            Json(json!({ "ok": true, "name": name, "version": body.version })).into_response()
        }
        Err(e) => io_error_response("rollback binary", e),
    }
}

/// GET /api/dag/binaries/:name/versions/:v/binary.bin — the pinned binary
/// binary. Bodies are ≤32 MiB by construction, so the full byte vector
/// is shipped in one response (no streaming).
pub async fn download(
    State(_state): State<Arc<AppState>>,
    Path((name, v)): Path<(String, String)>,
) -> Response {
    let version = match v.parse::<u32>() {
        Ok(v) => v,
        Err(_) => return error_400(format!("bad version: {v}")),
    };
    let Some(root) = pool_root() else {
        return error_500("dag binary pool root not configured".to_string());
    };
    let bytes = match std::fs::read(binary_bin(&root, &name, version)) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return error_404(&format!("unknown binary version: {name}/v{version}"))
        }
        Err(e) => return error_500(format!("read binary module: {e}")),
    };
    // The name passed charset validation on write, so the disposition
    // filename is header-safe by construction.
    let disposition = format!("attachment; filename=\"{name}-v{version}.binary\"");
    let len = bytes.len();
    let mut response = (StatusCode::OK, Body::from(bytes)).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    headers.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&len.to_string()).expect("usize is a valid header"),
    );
    if let Ok(value) = HeaderValue::from_str(&disposition) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    response
}
