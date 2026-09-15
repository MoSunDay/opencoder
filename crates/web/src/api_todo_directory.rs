//! Immutable TODO directory versions, shared by Web and Control.
use crate::{api_todo_util::*, AppState};
use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
    Json,
};
use opencoder_core::share_fs::{self, read_json_opt, todo_dir, todo_meta_path, todo_version_dir};
use opencoder_todos::directory::{self, Diagnostic, Files};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{fs::File, path::Path as FsPath, sync::Arc};

pub fn revision(meta: &Value) -> String {
    meta.to_string()
}

/// OS locks release on process exit and serialize all template publishers.
#[allow(clippy::result_large_err)]
pub(crate) fn lock(root: &FsPath, name: &str) -> Result<File, Response> {
    share_fs::validate_share_name(name).map_err(error_400)?;
    let locks = root.join(".todo-locks");
    share_fs::durable_create_dir_all(&locks)
        .map_err(|e| error_500(format!("template lock: {e:#}")))?;
    let file = File::options()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(locks.join(name))
        .map_err(|e| error_500(e.to_string()))?;
    file.try_lock()
        .map_err(|e| error_409(&format!("模板正在修改，请重试: {e}")))?;
    Ok(file)
}

pub fn invalid(errors: Vec<Diagnostic>) -> Response {
    (
        axum::http::StatusCode::BAD_REQUEST,
        Json(json!({"error":"文件不符合 TODO 框架要求","diagnostics":errors})),
    )
        .into_response()
}

fn diagnostic(path: &str, error: impl ToString) -> Diagnostic {
    Diagnostic {
        path: path.into(),
        message: error.to_string(),
        line: 1,
        column: 1,
    }
}

#[allow(clippy::result_large_err)]
fn check_files(root: &FsPath, files: &Files) -> Result<(), Response> {
    let (_, env) = directory::decode(files).map_err(invalid)?;
    if let Some(env) = env {
        let path = share_fs::env_context_path(root, &env)
            .map_err(|e| invalid(vec![diagnostic("env.json", e)]))?;
        let context = read_json_opt(&path)
            .map_err(|e| invalid(vec![diagnostic("env.json", e)]))?
            .ok_or_else(|| invalid(vec![diagnostic("env.json", format!("环境不存在: {env}"))]))?;
        opencoder_todos::domain::env_vars_from_context(&context)
            .map_err(|e| invalid(vec![diagnostic("env.json", e)]))?;
        if let Some(tools) = context.get("tools") {
            let tools = tools
                .as_array()
                .ok_or_else(|| invalid(vec![diagnostic("env.json", "环境 tools 必须是数组")]))?;
            for tool in tools {
                let reference = tool
                    .as_str()
                    .ok_or_else(|| invalid(vec![diagnostic("env.json", "工具引用必须是字符串")]))?;
                share_fs::resolve_tool_ref(root, reference)
                    .map_err(|e| invalid(vec![diagnostic("env.json", e)]))?;
            }
        }
    }
    Ok(())
}

#[allow(clippy::result_large_err)]
fn input_files(body: &Value) -> Result<Files, Response> {
    if let Some(files) = body.get("files") {
        return serde_json::from_value(files.clone())
            .map_err(|e| invalid(vec![diagnostic("workflow.json", e)]));
    }
    let spec = serde_json::from_value(body.get("spec").cloned().unwrap_or(Value::Null))
        .map_err(|e| invalid(vec![diagnostic("workflow.json", e)]))?;
    directory::encode(&spec, None).map_err(|e| invalid(vec![diagnostic("workflow.json", e)]))
}

pub async fn create(State(state): State<Arc<AppState>>, Json(body): Json<Value>) -> Response {
    let (_, root) = match share_root(&state.workdir).await {
        Ok(pair) => pair,
        Err(e) => return error_500(e.to_string()),
    };
    let name = body["name"].as_str().unwrap_or("");
    let _guard = match lock(&root, name) {
        Ok(file) => file,
        Err(response) => return response,
    };
    let dir = match todo_dir(&root, name) {
        Ok(dir) => dir,
        Err(e) => return error_400(e.to_string()),
    };
    if dir.exists() {
        return error_409("模板已存在");
    }
    let files = match input_files(&body) {
        Ok(files) => files,
        Err(response) => return response,
    };
    if let Err(response) = check_files(&root, &files) {
        return response;
    }
    let meta = json!({"name":name,"description":body["description"].as_str().unwrap_or(""),"current":"v1",
        "versions":[{"version":"v1","note":body["note"].as_str().unwrap_or(""),"created_at":now_ms()}]});
    // Publish the whole template including metadata in one rename.
    let parent = root.join("todo");
    let staging = parent.join(format!(".create-{}", ulid::Ulid::new()));
    let result = (|| -> anyhow::Result<()> {
        directory::write_new(&staging.join("v1"), &files)?;
        share_fs::atomic_write_json(&staging.join("todo.json"), &meta)?;
        std::fs::rename(&staging, &dir)?;
        File::open(&parent)?.sync_all()?;
        Ok(())
    })();
    match result {
        Ok(()) => Json(json!({"template":meta,"revision":revision(&meta)})).into_response(),
        Err(e) => error_500(format!("创建目录失败: {e:#}")),
    }
}

pub async fn save(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let (_, root) = match share_root(&state.workdir).await {
        Ok(pair) => pair,
        Err(e) => return error_500(e.to_string()),
    };
    let _guard = match lock(&root, &name) {
        Ok(file) => file,
        Err(response) => return response,
    };
    let path = match todo_meta_path(&root, &name) {
        Ok(path) => path,
        Err(e) => return error_400(e.to_string()),
    };
    let mut meta = match read_json_opt(&path) {
        Ok(Some(meta)) => meta,
        Ok(None) => return error_404("模板不存在"),
        Err(e) => return error_500(e.to_string()),
    };
    let edits =
        body.get("files").is_some() || body.get("spec").is_some() || body.get("binding").is_some();
    if body["expected_revision"]
        .as_str()
        .is_some_and(|expected| expected != revision(&meta))
        || body["expected_current"]
            .as_str()
            .is_some_and(|expected| Some(expected) != meta["current"].as_str())
    {
        return error_409("模板已修改，请重新加载并合并草稿");
    }
    if edits
        && body["expected_revision"].as_str().is_none()
        && body["expected_current"].as_str().is_none()
    {
        return error_409("保存需要 expected_revision 或 expected_current");
    }
    let source = body["source_version"]
        .as_str()
        .or(meta["current"].as_str())
        .unwrap_or("");
    let versions = match meta["versions"].as_array() {
        Some(v) => v,
        None => return error_500("todo.json versions 格式错误".into()),
    };
    if !versions.iter().any(|v| v["version"] == source) {
        return error_404("源版本不存在");
    }
    let source_path = match todo_version_dir(&root, &name, source) {
        Ok(path) => path,
        Err(e) => return error_400(e.to_string()),
    };
    let mut files = if body.get("files").is_some() || body.get("spec").is_some() {
        match input_files(&body) {
            Ok(files) => files,
            Err(response) => return response,
        }
    } else {
        match directory::read_files(&source_path) {
            Ok(files) => files,
            Err(e) => return invalid(vec![diagnostic(source, format!("{e:#}"))]),
        }
    };
    if body.get("spec").is_some() && body.get("binding").is_none() {
        match read_json_opt(&source_path.join("env.json")) {
            Ok(Some(binding)) => {
                files.insert("env.json".into(), binding.to_string());
            }
            Ok(None) => {}
            Err(e) => return invalid(vec![diagnostic("env.json", e)]),
        }
    }
    if let Some(binding) = body.get("binding") {
        files.insert("env.json".into(), binding.to_string());
    }
    if let Err(response) = check_files(&root, &files) {
        return response;
    }
    let dir = path.parent().expect("template parent");
    // Include unpublished directories left by a failed metadata write.
    let next = next_version(&share_fs::list_child_dirs(dir));
    if let Err(e) = directory::write_new(&dir.join(&next), &files) {
        return error_500(format!("保存版本失败: {e:#}"));
    }
    meta["versions"].as_array_mut().expect("validated versions").push(json!({"version":next,"note":body["note"].as_str().unwrap_or(""),"created_at":now_ms()}));
    meta["current"] = json!(next);
    if let Err(e) = share_fs::atomic_write_json(&path, &meta) {
        return error_500(format!("发布版本失败: {e:#}"));
    }
    Json(json!({"version":next,"template":meta,"revision":revision(&meta)})).into_response()
}

#[derive(Default, Deserialize)]
pub struct FilesQuery {
    pub path: Option<String>,
    pub tree: Option<bool>,
}

pub async fn files(
    State(state): State<Arc<AppState>>,
    Path((name, version)): Path<(String, String)>,
    Query(query): Query<FilesQuery>,
) -> Response {
    let (_, root) = match share_root(&state.workdir).await {
        Ok(pair) => pair,
        Err(e) => return error_500(e.to_string()),
    };
    let dir = match todo_version_dir(&root, &name, &version) {
        Ok(dir) => dir,
        Err(e) => return error_400(e.to_string()),
    };
    let meta = match read_json_opt(&dir.parent().expect("template").join("todo.json")) {
        Ok(Some(meta)) => meta,
        Ok(None) => return error_404("模板不存在"),
        Err(e) => return error_500(e.to_string()),
    };
    if !meta["versions"]
        .as_array()
        .is_some_and(|versions| versions.iter().any(|v| v["version"] == version))
    {
        return error_404("版本不存在");
    }
    let files = match directory::read_files(&dir) {
        Ok(files) => files,
        Err(e) => return invalid(vec![diagnostic(&version, format!("{e:#}"))]),
    };
    let mut errors = directory::validate(&files);
    if errors.is_empty() {
        if let Err(response) = check_files(&root, &files) {
            return response;
        }
    }
    let entries: Vec<_> = files
        .iter()
        .map(|(path, text)| json!({"path":path,"bytes":text.len()}))
        .collect();
    let mut result = json!({"version":version,"revision":revision(&meta),"entries":entries,"diagnostics":std::mem::take(&mut errors)});
    if let Some(path) = query.path {
        let Some(text) = files.get(&path) else {
            return error_404("文件不存在");
        };
        result["path"] = json!(path);
        result["content"] = json!(text);
    } else if query.tree != Some(true) {
        result["files"] = json!(files);
    }
    Json(result).into_response()
}

pub async fn validate_files(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Response {
    let (_, root) = match share_root(&state.workdir).await {
        Ok(pair) => pair,
        Err(e) => return error_500(e.to_string()),
    };
    let files = match input_files(&body) {
        Ok(files) => files,
        Err(response) => return response,
    };
    match check_files(&root, &files) {
        Ok(()) => Json(json!({"valid":true})).into_response(),
        Err(response) => response,
    }
}
