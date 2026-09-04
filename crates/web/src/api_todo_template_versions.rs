//! `/api/todo/templates` — version & template lifecycle handlers, split from
//! `api_todo_templates` to keep both files small:
//!
//! - `new_version`: fork a version (context verbatim + env binding carry +
//!   `current` flip),
//! - `delete_version`: drop one non-current version and prune `todo.json`,
//! - `delete_template`: drop the whole template tree.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use opencoder_core::share_fs::{
    todo_context_path, todo_env_binding_path, todo_meta_path, todo_version_dir, validate_share_name,
};

use crate::api_todo_templates::{name_or_resp, read_meta};
use crate::api_todo_util::{
    error_400, error_404, error_409, error_500, next_version, now_ms, share_root,
};
use crate::AppState;

/// POST /api/todo/templates/:name/new-version — fork the current (or
/// explicit `source_version`) version into `v{max+1}`: context bytes are
/// copied verbatim, an env binding (if any) rides along, and `current`
/// flips to the new version.
pub async fn new_version(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let root = match share_root(&state.workdir).await {
        Ok((_, root)) => root,
        Err(e) => return error_500(format!("share root: {e:#}")),
    };
    if let Err(resp) = name_or_resp(&root, &name) {
        return resp;
    }
    let mut meta = match read_meta(&root, &name).await {
        Ok(Some(meta)) => meta,
        Ok(None) => return error_404(&format!("模板不存在: {name}")),
        Err(resp) => return resp,
    };
    let source = body
        .get("source_version")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            meta.get("current")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| "v1".into());
    let source_context = match todo_context_path(&root, &name, &source) {
        Ok(p) => p,
        Err(e) => return error_400(format!("{e:#}")),
    };
    let context_bytes = match tokio::fs::read(&source_context).await {
        Ok(bytes) => bytes,
        Err(_) => return error_404(&format!("版本不存在: {name}/{source}")),
    };
    let existing: Vec<String> = meta
        .get("versions")
        .and_then(Value::as_array)
        .map(|versions| {
            versions
                .iter()
                .filter_map(|v| v.get("version").and_then(Value::as_str).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let next = next_version(&existing);
    let target_context = match todo_context_path(&root, &name, &next) {
        Ok(p) => p,
        Err(e) => return error_400(format!("{e:#}")),
    };
    if let Err(e) = opencoder_core::share_fs::atomic_write(&target_context, &context_bytes) {
        return error_500(format!("写入 context.json 失败: {e:#}"));
    }
    if let Ok(source_binding) = todo_env_binding_path(&root, &name, &source) {
        if let Ok(bytes) = tokio::fs::read(&source_binding).await {
            let target_binding = match todo_env_binding_path(&root, &name, &next) {
                Ok(p) => p,
                Err(e) => return error_400(format!("{e:#}")),
            };
            if let Err(e) = opencoder_core::share_fs::atomic_write(&target_binding, &bytes) {
                return error_500(format!("写入 env.json 失败: {e:#}"));
            }
        }
    }
    let note = body.get("note").and_then(Value::as_str).unwrap_or("");
    meta["versions"]
        .as_array_mut()
        .unwrap_or(&mut Vec::new())
        .push(json!({ "version": next, "note": note, "created_at": now_ms() }));
    meta["current"] = json!(next);
    let meta_path = match todo_meta_path(&root, &name) {
        Ok(p) => p,
        Err(e) => return error_400(format!("{e:#}")),
    };
    if let Err(e) = opencoder_core::share_fs::atomic_write_json(&meta_path, &meta) {
        return error_500(format!("写入 todo.json 失败: {e:#}"));
    }
    Json(json!({ "version": next })).into_response()
}

/// DELETE /api/todo/templates/:name/:version — drop one version (409 when it
/// is `current`). The metadata version list is pruned in the same call so
/// `todo.json` never advertises a directory that is gone.
pub async fn delete_version(
    State(state): State<Arc<AppState>>,
    Path((name, version)): Path<(String, String)>,
) -> Response {
    let root = match share_root(&state.workdir).await {
        Ok((_, root)) => root,
        Err(e) => return error_500(format!("share root: {e:#}")),
    };
    if let Err(resp) = name_or_resp(&root, &name) {
        return resp;
    }
    if let Err(e) = validate_share_name(&version) {
        return error_400(e);
    }
    let dir = match todo_version_dir(&root, &name, &version) {
        Ok(d) => d,
        Err(e) => return error_400(format!("{e:#}")),
    };
    if !dir.exists() {
        return error_404(&format!("版本不存在: {name}/{version}"));
    }
    let mut meta = match read_meta(&root, &name).await {
        Ok(Some(meta)) => meta,
        Ok(None) => return error_404(&format!("模板不存在: {name}")),
        Err(resp) => return resp,
    };
    if meta.get("current").and_then(Value::as_str) == Some(version.as_str()) {
        return error_409(&format!("不能删除 current 版本 {version}"));
    }
    if let Err(e) = tokio::fs::remove_dir_all(&dir).await {
        return error_500(format!("删除版本失败: {e:#}"));
    }
    if let Some(versions) = meta["versions"].as_array_mut() {
        versions.retain(|v| v.get("version").and_then(Value::as_str) != Some(version.as_str()));
    }
    let meta_path = match todo_meta_path(&root, &name) {
        Ok(p) => p,
        Err(e) => return error_400(format!("{e:#}")),
    };
    if let Err(e) = opencoder_core::share_fs::atomic_write_json(&meta_path, &meta) {
        return error_500(format!("写入 todo.json 失败: {e:#}"));
    }
    Json(json!({ "ok": true })).into_response()
}

/// DELETE /api/todo/templates/:name — drop the whole template (all versions
/// plus metadata).
pub async fn delete_template(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Response {
    let root = match share_root(&state.workdir).await {
        Ok((_, root)) => root,
        Err(e) => return error_500(format!("share root: {e:#}")),
    };
    let dir = match name_or_resp(&root, &name) {
        Ok(dir) => dir,
        Err(resp) => return resp,
    };
    if !dir.exists() {
        return error_404(&format!("模板不存在: {name}"));
    }
    match tokio::fs::remove_dir_all(&dir).await {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(e) => error_500(format!("删除模板失败: {e:#}")),
    }
}
