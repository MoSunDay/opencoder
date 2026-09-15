//! `/api/todo/templates` — TODO template CRUD over the share tree:
//!
//! ```text
//! <share>/todo/<name>/todo.json                  # {"name","description","current","versions":[...]}
//! <share>/todo/<name>/<version>/workflow.json    # Directory manifest; Markdown task files
//! <share>/todo/<name>/<version>/env.json         # {"env":"<env-name>"|null}
//! ```
//!
//! Directory versions are immutable; new-version validates the entire file set.

use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Map, Value};

use opencoder_core::share_fs::{
    list_child_dirs, read_json_opt, todo_dir, todo_env_binding_path, todo_meta_path,
    validate_share_name,
};

use crate::api_todo_util::{error_400, error_404, error_409, error_500, share_root};
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
#[allow(clippy::result_large_err)] // Return the already-built Axum response at this HTTP boundary; boxing adds an allocation per error.
pub(crate) async fn read_meta(
    root: &std::path::Path,
    name: &str,
) -> Result<Option<Value>, Response> {
    let path = todo_meta_path(root, name).map_err(|e| error_400(format!("{e:#}")))?;
    match read_json_opt(&path) {
        Ok(Some(meta)) => {
            let versions = meta["versions"].as_array();
            let valid = meta.is_object()
                && meta["current"].as_str().is_some_and(|current| {
                    versions.is_some_and(|entries| {
                        let names: Vec<_> = entries
                            .iter()
                            .filter_map(|entry| entry["version"].as_str())
                            .collect();
                        names.len() == entries.len()
                            && names.contains(&current)
                            && names.iter().all(|name| validate_share_name(name).is_ok())
                            && names
                                .iter()
                                .collect::<std::collections::BTreeSet<_>>()
                                .len()
                                == names.len()
                    })
                });
            if !valid {
                return Err(error_400(format!(
                    "{name}/todo.json: current 或 versions 格式错误"
                )));
            }
            Ok(Some(meta))
        }
        Ok(None) => Ok(None),
        Err(e) => Err(error_400(format!("{name}/todo.json: {e:#}"))),
    }
}

/// Read one version's env binding: `None` when the file is absent (unbound).
#[allow(clippy::result_large_err)] // Return the already-built Axum response at this HTTP boundary; boxing adds an allocation per error.
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
/// `todo.json`; unreadable published metadata is reported explicitly.
pub async fn list_templates(State(state): State<Arc<AppState>>) -> Response {
    let root = match share_root(&state.workdir).await {
        Ok((_, root)) => root,
        Err(e) => return error_500(format!("share root: {e:#}")),
    };
    let mut templates = Vec::new();
    for name in list_child_dirs(&root.join("todo"))
        .into_iter()
        .filter(|name| !name.starts_with('.'))
    {
        match read_meta(&root, &name).await {
            Ok(Some(mut meta)) => {
                meta["name"] = json!(name);
                templates.push(meta);
            }
            Ok(None) => return error_400(format!("{name}/todo.json: 缺少模板元数据")),
            Err(response) => return response,
        }
    }
    Json(json!({ "templates": templates })).into_response()
}

/// POST /api/todo/templates — create a template with its `v1` context. The
/// spec is parsed AND domain-validated (agent names, dependency cycles,
/// path-safe ids) before anything is written.
pub async fn create_template(state: State<Arc<AppState>>, body: Json<Value>) -> Response {
    crate::api_todo_directory::create(state, body).await
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
                Ok(None) => Value::Null,
                Err(response) => return response,
            };
            env_by_version.insert(version.to_string(), binding);
        }
    }
    Json(json!({ "template": meta, "revision":crate::api_todo_directory::revision(&meta), "env_by_version": env_by_version })).into_response()
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
    let _guard = match crate::api_todo_directory::lock(&root, &name) {
        Ok(file) => file,
        Err(response) => return response,
    };
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
    let (_, root) = match share_root(&state.workdir).await {
        Ok(pair) => pair,
        Err(e) => return error_500(e.to_string()),
    };
    let path = match opencoder_core::share_fs::todo_version_dir(&root, &name, &version) {
        Ok(path) => path,
        Err(e) => return error_400(e.to_string()),
    };
    if !path.is_dir() {
        return error_404("版本不存在");
    }
    match opencoder_todos::directory::load(&path) {
        Ok(spec) => Json(json!(spec)).into_response(),
        Err(e) => error_400(format!("文件不符合 TODO 框架要求: {e:#}")),
    }
}

/// Historical writes are rejected; publish edits through new-version.
pub async fn put_context() -> Response {
    error_409("模板版本只读，请通过 new-version 保存新版本")
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

/// Environment bindings are frozen with their version.
pub async fn put_env_binding() -> Response {
    error_409("环境绑定随模板版本冻结，请通过 new-version 保存新版本")
}
