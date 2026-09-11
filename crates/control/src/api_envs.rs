//! OpenCoder environment snapshots: complete config bundles under ~/.opencoder/envs.
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Map, Value};
use std::path::PathBuf;
use std::sync::Arc;

use crate::AppState;
use opencoder_core::config::envs::{
    active_env, create_env, delete_env, env_dir, list_envs, recapture_env, set_active_env_checked,
    validate_env_name,
};

const FILES: [(&str, &str); 5] = [
    ("config", "config.json"),
    ("mcp_servers", "mcp.json"),
    ("cli", "cli.json"),
    ("skills", "skills.json"),
    ("autopilot", "ap.json"),
];

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({"ok": false, "error": message.into()}))).into_response()
}

#[allow(clippy::result_large_err)]
fn env_path(name: &str) -> Result<PathBuf, Response> {
    validate_env_name(name).map_err(|e| error(StatusCode::BAD_REQUEST, e))?;
    env_dir(name).ok_or_else(|| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "cannot resolve ~/.opencoder",
        )
    })
}

fn read_json(path: &std::path::Path) -> Result<Value, std::io::Error> {
    match std::fs::read_to_string(path) {
        Ok(raw) if raw.trim().is_empty() => Ok(json!({})),
        Ok(raw) => serde_json::from_str(&raw)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(e) => Err(e),
    }
}

fn redact_and_mask(value: &Value) -> Value {
    opencoder_core::config::redact::redact_json(value)
}

/// Preserve credentials when a client sends the masked value returned by GET.
fn preserve_masked(existing: &Value, incoming: &mut Value) {
    match (existing, incoming) {
        (Value::Object(old), Value::Object(new)) => {
            for (key, value) in new.iter_mut() {
                if key == "api_key" && value.as_str().is_some_and(|s| s.contains("***")) {
                    if let Some(old_value) = old.get(key) {
                        *value = old_value.clone();
                    }
                } else if let Some(old_value) = old.get(key) {
                    preserve_masked(old_value, value);
                }
            }
        }
        (Value::Array(old), Value::Array(new)) => {
            for (index, value) in new.iter_mut().enumerate() {
                if let Some(old_value) = old.get(index) {
                    preserve_masked(old_value, value);
                }
            }
        }
        _ => {}
    }
}

pub async fn list(State(_state): State<Arc<AppState>>) -> Response {
    let active = active_env();
    let envs = list_envs()
        .into_iter()
        .map(|name| {
            json!({
                "name": name,
                "active": active.as_deref() == Some(name.as_str()),
            })
        })
        .collect::<Vec<_>>();
    Json(json!({"ok": true, "active": active, "envs": envs})).into_response()
}

pub async fn create(State(state): State<Arc<AppState>>, Json(body): Json<Value>) -> Response {
    let Some(name) = body
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return error(StatusCode::BAD_REQUEST, "name is required");
    };
    if let Err(e) = validate_env_name(name) {
        return error(StatusCode::BAD_REQUEST, e);
    }
    if list_envs().iter().any(|n| n == name) {
        return error(StatusCode::CONFLICT, format!("env already exists: {name}"));
    }
    let capture = body
        .get("capture_current")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    match create_env(name, &state.workdir, capture) {
        Ok(dir) => Json(json!({"ok": true, "name": name, "path": dir})).into_response(),
        Err(e) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("create env: {e:#}"),
        ),
    }
}

pub async fn detail(State(_state): State<Arc<AppState>>, Path(name): Path<String>) -> Response {
    let dir = match env_path(&name) {
        Ok(p) => p,
        Err(r) => return r,
    };
    if !dir.is_dir() {
        return error(StatusCode::NOT_FOUND, format!("unknown env: {name}"));
    }
    let mut files = Map::new();
    for (key, file) in FILES {
        match read_json(&dir.join(file)) {
            Ok(value) => {
                files.insert(key.to_string(), redact_and_mask(&value));
            }
            Err(e) => {
                return error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("read {file}: {e}"),
                )
            }
        }
    }
    Json(json!({"ok": true, "name": name, "active": active_env().as_deref() == Some(name.as_str()), "files": files})).into_response()
}

pub async fn update(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let dir = match env_path(&name) {
        Ok(p) => p,
        Err(r) => return r,
    };
    if !dir.is_dir() {
        return error(StatusCode::NOT_FOUND, format!("unknown env: {name}"));
    }
    let Some(input) = body.get("files").or(Some(&body)).and_then(Value::as_object) else {
        return error(StatusCode::BAD_REQUEST, "files must be an object");
    };
    let mut writes = Vec::new();
    for (key, file) in FILES {
        let Some(value) = input.get(key) else {
            continue;
        };
        if !value.is_object() {
            return error(
                StatusCode::BAD_REQUEST,
                format!("{key} must be a JSON object"),
            );
        }
        let path = dir.join(file);
        let existing = read_json(&path)
            .map_err(|e| error(StatusCode::BAD_REQUEST, format!("read {file}: {e}")));
        let Ok(existing) = existing else {
            return existing.unwrap_err();
        };
        let mut next = value.clone();
        preserve_masked(&existing, &mut next);
        writes.push((path, next));
    }
    for (path, value) in writes {
        let body = match serde_json::to_string_pretty(&value) {
            Ok(v) => v + "\n",
            Err(e) => return error(StatusCode::BAD_REQUEST, e.to_string()),
        };
        if let Err(e) = opencoder_core::config::envs::write_config_save_public(&path, &body) {
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("write {}: {e}", path.display()),
            );
        }
    }
    if active_env().as_deref() == Some(name.as_str()) {
        state.reload_agents().await;
    }
    Json(json!({"ok": true, "name": name})).into_response()
}

pub async fn activate(State(state): State<Arc<AppState>>, Json(body): Json<Value>) -> Response {
    let target = match body.get("active") {
        Some(Value::Null) | None => None,
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
        _ => {
            return error(
                StatusCode::BAD_REQUEST,
                "active must be an env name or null",
            )
        }
    };
    if let Some(name) = &target {
        if !list_envs().iter().any(|n| n == name) {
            return error(StatusCode::NOT_FOUND, format!("unknown env: {name}"));
        }
    }
    match set_active_env_checked(target.as_deref(), &state.workdir) {
        Ok(()) => {
            state.reload_agents().await;
            Json(json!({"ok": true, "active": target})).into_response()
        }
        Err(e) => error(StatusCode::BAD_REQUEST, e.to_string()),
    }
}

pub async fn recapture(State(state): State<Arc<AppState>>, Path(name): Path<String>) -> Response {
    match recapture_env(&name, &state.workdir) {
        Ok(()) => {
            if active_env().as_deref() == Some(name.as_str()) {
                state.reload_agents().await;
            }
            Json(json!({"ok": true, "name": name})).into_response()
        }
        Err(e) => error(StatusCode::BAD_REQUEST, e.to_string()),
    }
}

pub async fn delete(State(state): State<Arc<AppState>>, Path(name): Path<String>) -> Response {
    let was_active = active_env().as_deref() == Some(name.as_str());
    match delete_env(&name) {
        Ok(()) => {
            if was_active {
                state.reload_agents().await;
            }
            Json(json!({"ok": true, "name": name})).into_response()
        }
        Err(e) => error(StatusCode::NOT_FOUND, e.to_string()),
    }
}
