//! `/api/agents/resources/:cat` — the four shared, independently versioned
//! pools (`prompts|skills|tools|memory`) that agent cards reference by
//! name. Writes go through `opencoder_agents` (atomic temp-dir + rename
//! version swaps); reads through `opencoder_core::agent`. ReloadConfig fans
//! out only when the ACTIVE card's chain names the written resource (see
//! [`crate::api_agents::active_chain_references`]) — every other write is
//! a silent disk write.

use std::io;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde::Deserialize;
use serde_json::{json, Value};

use opencoder_agents::{rollback_resource, save_resource_version, VersionFile};
use opencoder_core::agent::{
    category_dir, list_agents, list_resources, read_agent_meta, read_resource_meta,
    resource_version_dir, validate_resource_name, AGENT_CATEGORIES,
};

use crate::api_agents::{active_chain_references, fan_out_reload};
use crate::AppState;

/// Decoded payload cap per request (1.5 MiB) — the whole `files` array,
/// not per file.
const MAX_TOTAL_BYTES: usize = 1536 * 1024;

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

/// Map write-path io errors onto REST statuses (envs idiom): `NotFound` ⇒
/// 404, `AlreadyExists` ⇒ 409, `InvalidInput`/`InvalidData` ⇒ 400.
fn io_error_response(ctx: &str, e: io::Error) -> Response {
    match e.kind() {
        io::ErrorKind::NotFound => error_404(&format!("{ctx}: {e}")),
        io::ErrorKind::AlreadyExists => (
            StatusCode::CONFLICT,
            Json(json!({ "ok": false, "error": format!("{ctx}: {e}") })),
        )
            .into_response(),
        io::ErrorKind::InvalidInput | io::ErrorKind::InvalidData => {
            error_400(format!("{ctx}: {e}"))
        }
        _ => error_500(format!("{ctx}: {e}")),
    }
}

fn unknown_category(cat: &str) -> Option<Response> {
    (!AGENT_CATEGORIES.contains(&cat))
        .then(|| error_400(format!("unknown resource category: {cat}")))
}

/// ReloadConfig only when the ACTIVE card's chain names this resource.
async fn maybe_fan_out(state: &AppState, cat: &str, resource: &str) {
    if active_chain_references(cat, resource) {
        fan_out_reload(state).await;
    }
}

/// A file path is safe when non-empty, relative (no leading `/`), free of
/// `..`/`.` segments and empty segments, and ≤64 segments deep — checked
/// before any filesystem work happens.
fn safe_rel_path(path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Err("file path cannot be empty".to_string());
    }
    if path.starts_with('/') {
        return Err(format!("file path cannot be absolute: {path}"));
    }
    let segments: Vec<&str> = path.split('/').collect();
    if segments.len() > 64 {
        return Err(format!("file path too deep (>64 segments): {path}"));
    }
    if segments.iter().any(|s| s.is_empty()) {
        return Err(format!("file path has an empty segment: {path}"));
    }
    if segments.iter().any(|s| *s == ".." || *s == ".") {
        return Err(format!(
            "file path must stay inside the version dir (no `..`): {path}"
        ));
    }
    Ok(())
}

/// Category-specific file shapes: prompts are exactly the three section
/// files, memory a single `memory.md`, skills `SKILL.md`-bearing
/// (`<skill>/SKILL.md` or `<skill>.md`); tools accept any safe path.
fn check_shape(cat: &str, path: &str) -> Result<(), String> {
    let ok = match cat {
        "prompts" => matches!(path, "soul.md" | "how.md" | "output.md"),
        "memory" => path == "memory.md",
        "skills" => {
            let segments: Vec<&str> = path.split('/').collect();
            (segments.len() == 2 && segments[1] == "SKILL.md")
                || (segments.len() == 1 && path.ends_with(".md"))
        }
        _ => true, // tools: any safe path
    };
    if ok {
        Ok(())
    } else {
        Err(format!("path `{path}` is not a legal {cat} file"))
    }
}

#[derive(Deserialize)]
pub struct SaveBody {
    pub name: String,
    pub files: Vec<SaveFile>,
}

#[derive(Deserialize)]
pub struct SaveFile {
    pub path: String,
    pub content_b64: String,
}

/// Decode + validate the whole request body before any filesystem work:
/// known category, legal name, safe & category-shaped paths, standard
/// base64, decoded total ≤ 1.5 MiB. Every failure is a 400 message.
fn decode_files(cat: &str, body: &SaveBody) -> Result<(String, Vec<VersionFile>), String> {
    if !AGENT_CATEGORIES.contains(&cat) {
        return Err(format!("unknown resource category: {cat}"));
    }
    let name = body.name.trim().to_string();
    validate_resource_name(cat, &name).map_err(|e| format!("invalid resource name: {e}"))?;
    let mut total: usize = 0;
    let mut files = Vec::with_capacity(body.files.len());
    for file in &body.files {
        safe_rel_path(&file.path)?;
        check_shape(cat, &file.path)?;
        let bytes = B64
            .decode(file.content_b64.as_bytes())
            .map_err(|e| format!("bad base64 in `{}`: {e}", file.path))?;
        total += bytes.len();
        if total > MAX_TOTAL_BYTES {
            return Err("decoded payload exceeds the 1.5 MiB cap".to_string());
        }
        files.push(VersionFile {
            rel_path: file.path.clone(),
            bytes,
        });
    }
    Ok((name, files))
}

/// GET /api/agents/resources/:cat — every pool entry with its current
/// version and full version history.
pub async fn list(State(_state): State<Arc<AppState>>, Path(cat): Path<String>) -> Response {
    if let Some(resp) = unknown_category(&cat) {
        return resp;
    }
    let resources: Vec<Value> = list_resources(&cat)
        .into_iter()
        .map(|name| {
            let meta = read_resource_meta(&cat, &name);
            json!({
                "name": name,
                "current": meta.as_ref().map(|m| m.current).unwrap_or(0),
                "versions": meta.map(|m| m.history).unwrap_or_default(),
            })
        })
        .collect();
    Json(json!({ "ok": true, "category": cat, "resources": resources })).into_response()
}

/// POST /api/agents/resources/:cat — create a resource at v1. 409 when a
/// resource with that name already exists (PUT is the version-bump path).
pub async fn create(
    State(state): State<Arc<AppState>>,
    Path(cat): Path<String>,
    Json(body): Json<SaveBody>,
) -> Response {
    let (name, files) = match decode_files(&cat, &body) {
        Ok(v) => v,
        Err(msg) => return error_400(msg),
    };
    if read_resource_meta(&cat, &name).is_some() {
        return (
            StatusCode::CONFLICT,
            Json(json!({
                "ok": false,
                "error": format!("resource already exists: {cat}/{name}"),
            })),
        )
            .into_response();
    }
    match save_resource_version(&cat, &name, &files) {
        Ok(version) => {
            maybe_fan_out(&state, &cat, &name).await;
            Json(json!({ "ok": true, "version": version })).into_response()
        }
        Err(e) => io_error_response("save resource", e),
    }
}

/// PUT /api/agents/resources/:cat/:name — same body as POST, but saves a
/// NEW version (`max(history ∪ current) + 1`; numbers are never reused).
/// Missing resource ⇒ 404.
pub async fn put_version(
    State(state): State<Arc<AppState>>,
    Path((cat, name)): Path<(String, String)>,
    Json(body): Json<SaveBody>,
) -> Response {
    if let Some(resp) = unknown_category(&cat) {
        return resp;
    }
    if read_resource_meta(&cat, &name).is_none() {
        return error_404(&format!("unknown resource: {cat}/{name}"));
    }
    let (name, files) = match decode_files(&cat, &body) {
        Ok(v) => v,
        Err(msg) => return error_400(msg),
    };
    match save_resource_version(&cat, &name, &files) {
        Ok(version) => {
            maybe_fan_out(&state, &cat, &name).await;
            Json(json!({ "ok": true, "version": version })).into_response()
        }
        Err(e) => io_error_response("save resource", e),
    }
}

/// GET /api/agents/resources/:cat/:name/meta — the resource's meta
/// (current version + history).
pub async fn meta(
    State(_state): State<Arc<AppState>>,
    Path((cat, name)): Path<(String, String)>,
) -> Response {
    if let Some(resp) = unknown_category(&cat) {
        return resp;
    }
    match read_resource_meta(&cat, &name) {
        Some(meta) => Json(json!({ "ok": true, "meta": meta })).into_response(),
        None => error_404(&format!("unknown resource: {cat}/{name}")),
    }
}

/// GET /api/agents/resources/:cat/:name/versions/:v/files/*path — one
/// file's bytes from a pinned version (base64 round-trip).
pub async fn read_file(
    State(_state): State<Arc<AppState>>,
    Path((cat, name, version, path)): Path<(String, String, u32, String)>,
) -> Response {
    if let Some(resp) = unknown_category(&cat) {
        return resp;
    }
    let Some(dir) = resource_version_dir(&cat, &name, version) else {
        return error_404(&format!("unknown resource: {cat}/{name}"));
    };
    if let Err(msg) = safe_rel_path(&path) {
        return error_400(msg);
    }
    match std::fs::read(dir.join(&path)) {
        Ok(bytes) => Json(json!({
            "ok": true,
            "path": path,
            "content_b64": B64.encode(&bytes),
            "size": bytes.len(),
        }))
        .into_response(),
        Err(_) => error_404(&format!("no such file: {cat}/{name}/v{version}/{path}")),
    }
}

#[derive(Deserialize)]
pub struct RollbackBody {
    pub version: u32,
}

/// POST /api/agents/resources/:cat/:name/rollback — point `current` back
/// at a historical version (pointer switch only; version dirs stay).
pub async fn rollback(
    State(state): State<Arc<AppState>>,
    Path((cat, name)): Path<(String, String)>,
    Json(body): Json<RollbackBody>,
) -> Response {
    if let Some(resp) = unknown_category(&cat) {
        return resp;
    }
    match rollback_resource(&cat, &name, body.version) {
        Ok(()) => {
            maybe_fan_out(&state, &cat, &name).await;
            Json(json!({ "ok": true, "current": body.version })).into_response()
        }
        Err(e) => io_error_response("rollback resource", e),
    }
}

/// Every card whose `current` names this pool resource (any category).
fn referencing_cards(cat: &str, resource: &str) -> Vec<String> {
    list_agents()
        .into_iter()
        .filter(|agent| {
            read_agent_meta(agent)
                .and_then(|m| match cat {
                    "prompts" => m.current.prompt,
                    "skills" => m.current.skills,
                    "tools" => m.current.tools,
                    "memory" => m.current.memory,
                    _ => None,
                })
                .is_some_and(|field| field == resource)
        })
        .collect()
}

/// DELETE /api/agents/resources/:cat/:name — 409 with the referencing
/// card names while ANY card points at the pool (cards are thin
/// references; deleting the pool would break them); otherwise the whole
/// `<cat>/<name>/` dir (all versions + meta) goes away.
pub async fn delete(
    State(state): State<Arc<AppState>>,
    Path((cat, name)): Path<(String, String)>,
) -> Response {
    if let Some(resp) = unknown_category(&cat) {
        return resp;
    }
    if read_resource_meta(&cat, &name).is_none() {
        return error_404(&format!("unknown resource: {cat}/{name}"));
    }
    let referenced_by = referencing_cards(&cat, &name);
    if !referenced_by.is_empty() {
        return (
            StatusCode::CONFLICT,
            Json(json!({
                "ok": false,
                "referenced_by": referenced_by,
                "error": format!("resource {cat}/{name} is referenced by agent cards"),
            })),
        )
            .into_response();
    }
    let Some(dir) = category_dir(&cat).map(|d| d.join(&name)) else {
        return error_404(&format!("unknown resource: {cat}/{name}"));
    };
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => {
            // Unreachable while referenced (409 above), kept for symmetry
            // with the reload policy: no card — let alone the active one —
            // can name this resource anymore.
            maybe_fan_out(&state, &cat, &name).await;
            Json(json!({ "ok": true, "deleted": name })).into_response()
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            error_404(&format!("unknown resource: {cat}/{name}"))
        }
        Err(e) => error_500(format!("delete resource: {e}")),
    }
}
