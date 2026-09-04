//! `/api/todo/envs` + `/api/todo/tools` — env-context and tool-share CRUD
//! over the NFS share tree:
//!
//! ```text
//! <share>/env/<name>/context.json        # {"name","description","tools":[],"env_vars":{}}
//! <share>/agent/tools/<version>/<tool>   # tool CLIs referenced as /agent/tools/v3/ffmpeg
//! ```
//!
//! `list_tools` additionally surfaces *importable* entries from the local
//! agents root (`<agents>/<agent>/tools/<version>/<tool>`) so a browser can
//! copy an agent-bundled tool into the share without shell access. All names
//! go through `share_fs::validate_share_name` (traversal-safe); every tool
//! reference in a saved env context must actually resolve to a file.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use opencoder_core::share_fs::{
    agent_tool_path, atomic_write, atomic_write_json, env_context_path, env_dir, list_child_dirs,
    list_child_files, read_json_opt, resolve_tool_ref, tool_ref, validate_share_name,
};

use crate::api_todo_util::{error_400, error_404, error_409, error_500, is_version, share_root};
use crate::AppState;

/// Resolved `(share root)` for the request, or a 500 response.
async fn root_or_500(state: &AppState) -> Result<std::path::PathBuf, Response> {
    share_root(&state.workdir)
        .await
        .map(|(_, root)| root)
        .map_err(|e| error_500(format!("share root: {e:#}")))
}

/// GET /api/todo/envs — every env dir that carries a parseable context.json
/// (dirs without one are skipped, never fatal: the tree may be mid-write).
pub async fn list_envs(State(state): State<Arc<AppState>>) -> Response {
    let root = match root_or_500(&state).await {
        Ok(root) => root,
        Err(resp) => return resp,
    };
    let mut envs = Vec::new();
    for name in list_child_dirs(&root.join("env")) {
        let Ok(path) = env_context_path(&root, &name) else {
            continue;
        };
        match read_json_opt(&path) {
            Ok(Some(mut ctx)) => {
                // The dir name is authoritative (the file may have been
                // copied around on the share).
                ctx["name"] = json!(name);
                envs.push(ctx);
            }
            _ => continue,
        }
    }
    Json(json!({ "envs": envs })).into_response()
}

/// POST /api/todo/envs — create an env context. 400 on a traversal-shaped
/// name, 409 when the env dir already exists (even without a context.json:
/// leftovers must not be silently overwritten).
pub async fn create_env(State(state): State<Arc<AppState>>, Json(body): Json<Value>) -> Response {
    let root = match root_or_500(&state).await {
        Ok(root) => root,
        Err(resp) => return resp,
    };
    let Some(name) = body.get("name").and_then(Value::as_str) else {
        return error_400("缺少 name 字段".into());
    };
    if let Err(e) = validate_share_name(name) {
        return error_400(e);
    }
    let dir = match env_dir(&root, name) {
        Ok(d) => d,
        Err(e) => return error_400(format!("{e:#}")),
    };
    if dir.exists() {
        return error_409(&format!("env 已存在: {name}"));
    }
    let context = json!({
        "name": name,
        "description": body.get("description").and_then(Value::as_str).unwrap_or(""),
        "tools": body.get("tools").cloned().unwrap_or(json!([])),
        "env_vars": body.get("env_vars").cloned().unwrap_or(json!({})),
    });
    let path = match env_context_path(&root, name) {
        Ok(p) => p,
        Err(e) => return error_400(format!("{e:#}")),
    };
    match atomic_write_json(&path, &context) {
        Ok(()) => Json(json!({ "ok": true, "name": name })).into_response(),
        Err(e) => error_500(format!("写入 env 失败: {e:#}")),
    }
}

/// GET /api/todo/envs/:name — the context JSON (404 when absent).
pub async fn get_env(State(state): State<Arc<AppState>>, Path(name): Path<String>) -> Response {
    let root = match root_or_500(&state).await {
        Ok(root) => root,
        Err(resp) => return resp,
    };
    if let Err(e) = validate_share_name(&name) {
        return error_400(e);
    }
    let path = match env_context_path(&root, &name) {
        Ok(p) => p,
        Err(e) => return error_400(format!("{e:#}")),
    };
    match read_json_opt(&path) {
        Ok(Some(mut ctx)) => {
            ctx["name"] = json!(name);
            Json(ctx).into_response()
        }
        Ok(None) => error_404(&format!("env 不存在: {name}")),
        Err(e) => error_500(format!("读取 env 失败: {e:#}")),
    }
}

/// PUT /api/todo/envs/:name — merge-patch semantics: absent keys keep their
/// stored value. Every entry of the final `tools` array must resolve to a
/// real file under the share (400 otherwise), so a saved env can never
/// reference a tool the runner would fail to find.
pub async fn update_env(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let root = match root_or_500(&state).await {
        Ok(root) => root,
        Err(resp) => return resp,
    };
    if let Err(e) = validate_share_name(&name) {
        return error_400(e);
    }
    let path = match env_context_path(&root, &name) {
        Ok(p) => p,
        Err(e) => return error_400(format!("{e:#}")),
    };
    let mut ctx = match read_json_opt(&path) {
        Ok(Some(ctx)) => ctx,
        Ok(None) => return error_404(&format!("env 不存在: {name}")),
        Err(e) => return error_500(format!("读取 env 失败: {e:#}")),
    };
    for key in ["description", "tools", "env_vars"] {
        if let Some(v) = body.get(key) {
            ctx[key] = v.clone();
        }
    }
    if let Some(list) = ctx.get("tools").and_then(Value::as_array) {
        for item in list {
            let Some(reference) = item.as_str() else {
                return error_400(format!("工具引用必须是字符串: {item}"));
            };
            if let Err(e) = resolve_tool_ref(&root, reference) {
                return error_400(format!("工具引用无法解析: {reference}: {e:#}"));
            }
        }
    }
    match atomic_write_json(&path, &ctx) {
        Ok(()) => Json(json!({ "ok": true, "name": name })).into_response(),
        Err(e) => error_500(format!("写入 env 失败: {e:#}")),
    }
}

/// DELETE /api/todo/envs/:name — remove the whole env dir.
pub async fn delete_env(State(state): State<Arc<AppState>>, Path(name): Path<String>) -> Response {
    let root = match root_or_500(&state).await {
        Ok(root) => root,
        Err(resp) => return resp,
    };
    if let Err(e) = validate_share_name(&name) {
        return error_400(e);
    }
    let dir = match env_dir(&root, &name) {
        Ok(d) => d,
        Err(e) => return error_400(format!("{e:#}")),
    };
    if !dir.exists() {
        return error_404(&format!("env 不存在: {name}"));
    }
    match tokio::fs::remove_dir_all(&dir).await {
        Ok(()) => Json(json!({ "ok": true, "name": name })).into_response(),
        Err(e) => error_500(format!("删除 env 失败: {e:#}")),
    }
}

/// Importable tool entry from the local agents root. The share copy wins the
/// sort; `agent`/`version`/`tool` identify the source for `import_tool`.
fn importable_entry(agent: &str, version: &str, tool: &str) -> Value {
    json!({
        "ref": tool_ref(version, tool),
        "source": "importable",
        "agent": agent,
        "version": version,
        "tool": tool,
    })
}

/// GET /api/todo/tools — union of tools already in the share (`source:
/// "share"`) and tools importable from the local agents root
/// (`source: "importable"`). Sorted by ref; the `active` marker is skipped.
pub async fn list_tools(State(state): State<Arc<AppState>>) -> Response {
    let root = match root_or_500(&state).await {
        Ok(root) => root,
        Err(resp) => return resp,
    };
    let mut tools: Vec<Value> = Vec::new();
    let share_tools = root.join("agent").join("tools");
    for version in list_child_dirs(&share_tools) {
        for tool in list_child_files(&share_tools.join(&version)) {
            tools.push(json!({ "ref": tool_ref(&version, &tool), "source": "share" }));
        }
    }
    if let Some(agents_root) = opencoder_core::agent::agents_dir() {
        for agent in list_child_dirs(&agents_root) {
            if agent == "active" {
                continue;
            }
            let versions_home = agents_root.join(&agent).join("tools");
            for version in list_child_dirs(&versions_home) {
                if !is_version(&version) {
                    continue;
                }
                for tool in list_child_files(&versions_home.join(&version)) {
                    tools.push(importable_entry(&agent, &version, &tool));
                }
            }
        }
    }
    tools.sort_by(|a, b| {
        a.get("ref")
            .and_then(Value::as_str)
            .cmp(&b.get("ref").and_then(Value::as_str))
    });
    Json(json!({ "tools": tools })).into_response()
}

/// POST /api/todo/tools/import — copy `<agents>/<agent>/tools/<version>/<tool>`
/// byte-for-byte into the share and return its canonical ref.
pub async fn import_tool(State(state): State<Arc<AppState>>, Json(body): Json<Value>) -> Response {
    let root = match root_or_500(&state).await {
        Ok(root) => root,
        Err(resp) => return resp,
    };
    let fields = ["agent", "version", "tool"];
    let mut parts: Vec<&str> = Vec::with_capacity(3);
    for field in fields {
        match body.get(field).and_then(Value::as_str) {
            Some(v) => parts.push(v),
            None => return error_400(format!("缺少 {field} 字段")),
        }
    }
    let [agent, version, tool] = [parts[0], parts[1], parts[2]];
    for (label, part) in [("agent", agent), ("version", version), ("tool", tool)] {
        if let Err(e) = validate_share_name(part) {
            return error_400(format!("{label}: {e}"));
        }
    }
    if !is_version(version) {
        return error_400(format!("version 必须形如 v<n>: {version}"));
    }
    let Some(agents_root) = opencoder_core::agent::agents_dir() else {
        return error_404("agents 目录不可用");
    };
    let source = agents_root
        .join(agent)
        .join("tools")
        .join(version)
        .join(tool);
    if !source.is_file() {
        return error_404(&format!("源工具不存在: {}", source.display()));
    }
    let bytes = match tokio::fs::read(&source).await {
        Ok(bytes) => bytes,
        Err(e) => return error_500(format!("读取源工具失败: {e:#}")),
    };
    let target = match agent_tool_path(&root, version, tool) {
        Ok(p) => p,
        Err(e) => return error_400(format!("{e:#}")),
    };
    match atomic_write(&target, &bytes) {
        Ok(()) => Json(json!({ "ok": true, "ref": tool_ref(version, tool) })).into_response(),
        Err(e) => error_500(format!("写入工具失败: {e:#}")),
    }
}
