//! `/api/todo/templates` — TODO template CRUD over the share tree:
//!
//! ```text
//! <share>/todo/<name>/todo.json                  # {"name","description","current","versions":[...]}
//! <share>/todo/<name>/<version>/context.json     # WorkflowSpec JSON (validated on write)
//! <share>/todo/<name>/<version>/env.json         # {"env":"<env-name>"|null}
//! ```
//!
//! Every write of a context re-runs `opencoder_todos::domain::validate_spec`
//! so the share can never hold a spec the runner would reject at dispatch
//! time. Version directories are `v<n>`; `new-version` copies the source
//! version's files verbatim and flips `current`.

use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Map, Value};

use opencoder_core::share_fs::{
    list_child_dirs, read_json_opt, todo_context_path, todo_dir, todo_env_binding_path,
    todo_meta_path, validate_share_name,
};
use opencoder_todos::WorkflowSpec;

use crate::api_todo_util::{error_400, error_404, error_409, error_500, now_ms, share_root};
use crate::AppState;

/// Validated `(name)` from the path, or a 400/500 response.
#[allow(clippy::result_large_err)] // Response is the natural error currency here
pub(crate) fn name_or_resp(root: &std::path::Path, name: &str) -> Result<PathBuf, Response> {
    if let Err(e) = validate_share_name(name) {
        return Err(error_400(e));
    }
    todo_dir(root, name).map_err(|e| error_400(format!("{e:#}")))
}

/// Read the template metadata; `Ok(None)` ⇒ unknown template (404 upstream).
pub(crate) async fn read_meta(
    root: &std::path::Path,
    name: &str,
) -> Result<Option<Value>, Response> {
    let path = todo_meta_path(root, name).map_err(|e| error_400(format!("{e:#}")))?;
    match read_json_opt(&path) {
        Ok(meta) => Ok(meta),
        Err(e) => Err(error_500(format!("读取模板元数据失败: {e:#}"))),
    }
}

/// Read one version's env binding: `None` when the file is absent (unbound).
async fn read_binding(
    root: &std::path::Path,
    name: &str,
    version: &str,
) -> Result<Option<Value>, Response> {
    let path =
        todo_env_binding_path(root, name, version).map_err(|e| error_400(format!("{e:#}")))?;
    match read_json_opt(&path) {
        Ok(binding) => Ok(binding),
        Err(e) => Err(error_500(format!("读取 env 绑定失败: {e:#}"))),
    }
}

/// GET /api/todo/templates — metadata of every template dir with a parseable
/// `todo.json` (mid-write/unreadable dirs are skipped, never fatal).
pub async fn list_templates(State(state): State<Arc<AppState>>) -> Response {
    let root = match share_root(&state.workdir).await {
        Ok((_, root)) => root,
        Err(e) => return error_500(format!("share root: {e:#}")),
    };
    let mut templates = Vec::new();
    for name in list_child_dirs(&root.join("todo")) {
        if let Ok(Some(mut meta)) = read_meta(&root, &name).await {
            meta["name"] = json!(name);
            templates.push(meta);
        }
    }
    Json(json!({ "templates": templates })).into_response()
}

/// POST /api/todo/templates — create a template with its `v1` context. The
/// spec is parsed AND domain-validated (agent names, dependency cycles,
/// path-safe ids) before anything is written.
pub async fn create_template(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Response {
    let root = match share_root(&state.workdir).await {
        Ok((_, root)) => root,
        Err(e) => return error_500(format!("share root: {e:#}")),
    };
    let Some(name) = body.get("name").and_then(Value::as_str) else {
        return error_400("缺少 name 字段".into());
    };
    if let Err(e) = validate_share_name(name) {
        return error_400(e);
    }
    let dir = match todo_dir(&root, name) {
        Ok(d) => d,
        Err(e) => return error_400(format!("{e:#}")),
    };
    if dir.exists() {
        return error_409(&format!("模板已存在: {name}"));
    }
    let Some(spec_value) = body.get("spec") else {
        return error_400("缺少 spec 字段".into());
    };
    let spec: WorkflowSpec = match serde_json::from_value(spec_value.clone()) {
        Ok(spec) => spec,
        Err(e) => return error_400(format!("{e:#}")),
    };
    if let Err(e) = opencoder_todos::domain::validate_spec(&spec) {
        return error_400(format!("{e:#}"));
    }
    let now = now_ms();
    let meta = json!({
        "name": name,
        "description": body.get("description").and_then(Value::as_str).unwrap_or(""),
        "current": "v1",
        "versions": [{
            "version": "v1",
            "note": body.get("note").and_then(Value::as_str).unwrap_or(""),
            "created_at": now,
        }],
    });
    let context_path = match todo_context_path(&root, name, "v1") {
        Ok(p) => p,
        Err(e) => return error_400(format!("{e:#}")),
    };
    let spec_json = match serde_json::to_value(&spec) {
        Ok(v) => v,
        Err(e) => return error_500(format!("序列化 spec 失败: {e:#}")),
    };
    if let Err(e) = opencoder_core::share_fs::atomic_write_json(&context_path, &spec_json) {
        return error_500(format!("写入 context.json 失败: {e:#}"));
    }
    let meta_path = match todo_meta_path(&root, name) {
        Ok(p) => p,
        Err(e) => return error_400(format!("{e:#}")),
    };
    if let Err(e) = opencoder_core::share_fs::atomic_write_json(&meta_path, &meta) {
        return error_500(format!("写入 todo.json 失败: {e:#}"));
    }
    Json(json!({ "template": meta })).into_response()
}

/// GET /api/todo/templates/:name — metadata plus the per-version env binding
/// map (`{"v1":"myenv"|null}`), so a browser renders bindings without N+1
/// requests.
pub async fn get_template(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
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
    meta["name"] = json!(name);
    let mut env_by_version = Map::new();
    if let Some(versions) = meta.get("versions").and_then(Value::as_array) {
        for entry in versions {
            let Some(version) = entry.get("version").and_then(Value::as_str) else {
                continue;
            };
            let binding = match read_binding(&root, &name, version).await {
                Ok(Some(binding)) => binding.get("env").cloned().unwrap_or(Value::Null),
                _ => Value::Null,
            };
            env_by_version.insert(version.to_string(), binding);
        }
    }
    Json(json!({ "template": meta, "env_by_version": env_by_version })).into_response()
}

/// GET /api/todo/templates/:name/todo.json — metadata only.
pub async fn get_meta(State(state): State<Arc<AppState>>, Path(name): Path<String>) -> Response {
    let root = match share_root(&state.workdir).await {
        Ok((_, root)) => root,
        Err(e) => return error_500(format!("share root: {e:#}")),
    };
    if let Err(resp) = name_or_resp(&root, &name) {
        return resp;
    }
    match read_meta(&root, &name).await {
        Ok(mut meta) => {
            if let Some(meta) = meta.as_mut() {
                meta["name"] = json!(name);
            }
            Json(json!({ "template": meta })).into_response()
        }
        Err(resp) => resp,
    }
}

/// PUT /api/todo/templates/:name/todo.json — merge-patch `description` and
/// `current` (a `current` outside the known versions is a 400, never a
/// dangling pointer).
pub async fn update_meta(
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
    if let Some(description) = body.get("description") {
        meta["description"] = description.clone();
    }
    if let Some(current) = body.get("current").and_then(Value::as_str) {
        let known = meta
            .get("versions")
            .and_then(Value::as_array)
            .map(|versions| {
                versions
                    .iter()
                    .any(|v| v.get("version").and_then(Value::as_str) == Some(current))
            })
            .unwrap_or(false);
        if !known {
            return error_400(format!("unknown version {current}"));
        }
        meta["current"] = json!(current);
    }
    let meta_path = match todo_meta_path(&root, &name) {
        Ok(p) => p,
        Err(e) => return error_400(format!("{e:#}")),
    };
    if let Err(e) = opencoder_core::share_fs::atomic_write_json(&meta_path, &meta) {
        return error_500(format!("写入 todo.json 失败: {e:#}"));
    }
    Json(json!({ "ok": true, "template": meta })).into_response()
}

/// GET /api/todo/templates/:name/:version/context.json — the stored spec.
pub async fn get_context(
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
    let path = match todo_context_path(&root, &name, &version) {
        Ok(p) => p,
        Err(e) => return error_400(format!("{e:#}")),
    };
    match read_json_opt(&path) {
        Ok(Some(context)) => Json(context).into_response(),
        Ok(None) => error_404(&format!("版本不存在: {name}/{version}")),
        Err(e) => error_500(format!("读取 context.json 失败: {e:#}")),
    }
}

/// PUT /api/todo/templates/:name/:version/context.json — replace the spec.
/// The body IS the WorkflowSpec JSON; parse + domain validation gate the
/// write so a broken spec can never replace a runnable one.
pub async fn put_context(
    State(state): State<Arc<AppState>>,
    Path((name, version)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Response {
    let root = match share_root(&state.workdir).await {
        Ok((_, root)) => root,
        Err(e) => return error_500(format!("share root: {e:#}")),
    };
    let dir = match name_or_resp(&root, &name) {
        Ok(dir) => dir,
        Err(resp) => return resp,
    };
    if let Err(e) = validate_share_name(&version) {
        return error_400(e);
    }
    if !dir.exists() {
        return error_404(&format!("模板不存在: {name}"));
    }
    let spec: WorkflowSpec = match serde_json::from_value(body) {
        Ok(spec) => spec,
        Err(e) => return error_400(format!("spec 解析失败: {e}")),
    };
    if let Err(e) = opencoder_todos::domain::validate_spec(&spec) {
        return error_400(format!("spec 校验失败: {e:#}"));
    }
    let path = match todo_context_path(&root, &name, &version) {
        Ok(p) => p,
        Err(e) => return error_400(format!("{e:#}")),
    };
    let spec_json = match serde_json::to_value(&spec) {
        Ok(v) => v,
        Err(e) => return error_500(format!("序列化 spec 失败: {e:#}")),
    };
    match opencoder_core::share_fs::atomic_write_json(&path, &spec_json) {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(e) => error_500(format!("写入 context.json 失败: {e:#}")),
    }
}

/// GET /api/todo/templates/:name/:version/env.json — `{"env":null}` when the
/// file is absent, else its stored content.
pub async fn get_env_binding(
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
    match read_binding(&root, &name, &version).await {
        Ok(Some(binding)) => Json(binding).into_response(),
        Ok(None) => Json(json!({ "env": null })).into_response(),
        Err(resp) => resp,
    }
}

/// PUT /api/todo/templates/:name/:version/env.json — bind (or clear) an env.
/// A non-empty target must exist as an env context; empty/null clears by
/// writing `{"env":null}` (an explicit tombstone beats a missing file for
/// NFS readers that cache directory listings).
pub async fn put_env_binding(
    State(state): State<Arc<AppState>>,
    Path((name, version)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Response {
    let root = match share_root(&state.workdir).await {
        Ok((_, root)) => root,
        Err(e) => return error_500(format!("share root: {e:#}")),
    };
    let dir = match name_or_resp(&root, &name) {
        Ok(dir) => dir,
        Err(resp) => return resp,
    };
    if let Err(e) = validate_share_name(&version) {
        return error_400(e);
    }
    if !dir.exists() {
        return error_404(&format!("模板不存在: {name}"));
    }
    let target = body.get("env").and_then(Value::as_str).unwrap_or("");
    if !target.is_empty() {
        if let Err(e) = validate_share_name(target) {
            return error_400(e);
        }
        let context_path = match opencoder_core::share_fs::env_context_path(&root, target) {
            Ok(p) => p,
            Err(e) => return error_400(format!("{e:#}")),
        };
        match read_json_opt(&context_path) {
            Ok(Some(_)) => {}
            Ok(None) => return error_400(format!("env 不存在: {target}")),
            Err(e) => return error_500(format!("读取 env 失败: {e:#}")),
        }
    }
    let path = match todo_env_binding_path(&root, &name, &version) {
        Ok(p) => p,
        Err(e) => return error_400(format!("{e:#}")),
    };
    let value = json!({ "env": if target.is_empty() { Value::Null } else { json!(target) } });
    match opencoder_core::share_fs::atomic_write_json(&path, &value) {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(e) => error_500(format!("写入 env.json 失败: {e:#}")),
    }
}
