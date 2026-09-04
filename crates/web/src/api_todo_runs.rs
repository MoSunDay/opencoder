//! `/api/todo/templates/:name/:version/run` + `/api/todo/workflows` — the
//! execution side of template management. A run resolves the stored spec,
//! applies the bound env (name + tool list stamped into `spec.metadata` after
//! verifying every referenced tool exists), then spawns an
//! `opencoder_todos::Runtime` against the shared store and returns the
//! workflow id immediately — progress is observable through the store-backed
//! endpoints (`/workflows`, `/workflows/:id/events`) which also see workflows
//! driven by the CLI in another process.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use opencoder_core::share_fs::{
    env_context_path, read_json_opt, resolve_tool_ref, todo_context_path, todo_env_binding_path,
    validate_share_name,
};
use opencoder_llm::{ChatClient, ChatStream};
use opencoder_todos::WorkflowSpec;

use crate::api_todo_util::{error_400, error_404, error_409, error_500, share_root};
use crate::AppState;

/// LLM client for a spawned runtime: injected in tests, real `ChatClient`
/// from the loaded config in production.
#[allow(clippy::result_large_err)] // Response is the natural error currency here
fn client_for(
    state: &AppState,
    config: &opencoder_core::Config,
) -> Result<Arc<dyn ChatStream>, Response> {
    if let Some(client) = state.client_override.clone() {
        return Ok(client);
    }
    let endpoint = config
        .resolve_endpoint()
        .map_err(|e| error_500(format!("endpoint: {e}")))?;
    ChatClient::new_with_read_timeout(
        &endpoint.base_url,
        &endpoint.api_key,
        &endpoint.headers,
        config.stream_idle_timeout(),
        config.network.proxy.as_deref(),
    )
    .map(|c| Arc::new(c) as Arc<dyn ChatStream>)
    .map_err(|e| error_500(format!("client: {e}")))
}

/// Read the version's spec and env binding. `(None binding)` ⇒ unbound.
async fn load_version(
    root: &std::path::Path,
    name: &str,
    version: &str,
) -> Result<(Value, Option<String>), Response> {
    let context_path =
        todo_context_path(root, name, version).map_err(|e| error_400(format!("{e:#}")))?;
    let context = match read_json_opt(&context_path) {
        Ok(Some(context)) => context,
        Ok(None) => return Err(error_404(&format!("版本不存在: {name}/{version}"))),
        Err(e) => return Err(error_500(format!("读取 context.json 失败: {e:#}"))),
    };
    let binding_path =
        todo_env_binding_path(root, name, version).map_err(|e| error_400(format!("{e:#}")))?;
    let env = match read_json_opt(&binding_path) {
        Ok(Some(binding)) => binding
            .get("env")
            .and_then(Value::as_str)
            .map(str::to_string)
            .filter(|env| !env.is_empty()),
        Ok(None) => None,
        Err(e) => return Err(error_500(format!("读取 env.json 失败: {e:#}"))),
    };
    Ok((context, env))
}

/// Apply the env binding to an in-memory spec: verify the env context and
/// every referenced tool exist, then stamp `metadata.env` / `metadata.env_tools`
/// (non-object metadata is replaced with an object — it is opaque to the
/// runner) and re-validate so the stored spec semantics stay intact.
#[allow(clippy::result_large_err)] // Response is the natural error currency here
fn apply_env(spec: &mut WorkflowSpec, root: &std::path::Path, env: &str) -> Result<(), Response> {
    if let Err(e) = validate_share_name(env) {
        return Err(error_400(e));
    }
    let context_path = env_context_path(root, env).map_err(|e| error_400(format!("{e:#}")))?;
    let env_context = match read_json_opt(&context_path) {
        Ok(Some(context)) => context,
        Ok(None) => return Err(error_400(format!("env 不存在: {env}"))),
        Err(e) => return Err(error_500(format!("读取 env 失败: {e:#}"))),
    };
    let tools = env_context.get("tools").cloned().unwrap_or(json!([]));
    if let Some(list) = tools.as_array() {
        for item in list {
            let Some(reference) = item.as_str() else {
                return Err(error_400(format!("env 工具引用必须是字符串: {item}")));
            };
            if resolve_tool_ref(root, reference).is_err() {
                return Err(error_400(format!("env 工具缺失: {reference}")));
            }
        }
    }
    if !spec.metadata.is_object() {
        spec.metadata = json!({});
    }
    spec.metadata["env"] = json!(env);
    spec.metadata["env_tools"] = tools;
    opencoder_todos::domain::validate_spec(spec)
        .map_err(|e| error_400(format!("spec 校验失败: {e:#}")))
}

/// POST /api/todo/templates/:name/:version/run — spawn the workflow and
/// return `{"workflow_id"}` immediately.
pub async fn run_template(
    State(state): State<Arc<AppState>>,
    Path((name, version)): Path<(String, String)>,
) -> Response {
    let (config, root) = match share_root(&state.workdir).await {
        Ok(pair) => pair,
        Err(e) => return error_500(format!("share root: {e:#}")),
    };
    for part in [&name, &version] {
        if let Err(e) = validate_share_name(part) {
            return error_400(e);
        }
    }
    let (context, env) = match load_version(&root, &name, &version).await {
        Ok(pair) => pair,
        Err(resp) => return resp,
    };
    let mut spec: WorkflowSpec = match serde_json::from_value(context) {
        Ok(spec) => spec,
        Err(e) => return error_400(format!("spec 解析失败: {e}")),
    };
    if let Err(e) = opencoder_todos::domain::validate_spec(&spec) {
        return error_400(format!("spec 校验失败: {e:#}"));
    }
    if let Some(env) = env.as_deref() {
        if let Err(resp) = apply_env(&mut spec, &root, env) {
            return resp;
        }
    }
    let client = match client_for(&state, &config) {
        Ok(client) => client,
        Err(resp) => return resp,
    };
    let workflow_id = format!("todos-{}", ulid::Ulid::new());
    let runtime = opencoder_todos::Runtime {
        store: state.store.clone(),
        client,
        config,
        workdir: state.workdir.clone(),
        debug_root: None,
        cancel: tokio_util::sync::CancellationToken::new(),
    };
    let id = workflow_id.clone();
    tokio::spawn(async move {
        if let Err(e) = runtime.run_new_with_id(spec, workflow_id.clone()).await {
            tracing::error!(workflow_id = %workflow_id, "todo run failed: {e:#}");
        }
    });
    Json(json!({ "workflow_id": id })).into_response()
}

#[derive(Deserialize, Default)]
pub struct WorkflowsQuery {
    pub limit: Option<u32>,
}

/// GET /api/todo/workflows?limit= — most-recent workflow summaries
/// (default 50, clamped to 1..=200).
pub async fn list_workflows(
    State(state): State<Arc<AppState>>,
    Query(q): Query<WorkflowsQuery>,
) -> Response {
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    match state.store.list_todo_workflows(limit).await {
        Ok(workflows) => Json(json!({ "workflows": workflows })).into_response(),
        Err(e) => error_500(format!("list workflows: {e:#}")),
    }
}

/// GET /api/todo/workflows/:id — the full record (spec + state JSON) plus
/// per-TODO item projections.
pub async fn get_workflow(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match state.store.get_todo_workflow(&id).await {
        Ok(Some(record)) => match state.store.list_todo_items(&id).await {
            Ok(items) => Json(json!({ "workflow": record, "items": items })).into_response(),
            Err(e) => error_500(format!("list items: {e:#}")),
        },
        Ok(None) => error_404(&format!("workflow 不存在: {id}")),
        Err(e) => error_500(format!("get workflow: {e:#}")),
    }
}

/// POST /api/todo/workflows/:id/interrupt — park a running workflow
/// (`workflow_interrupted`). Terminal workflows are refused with 409 before
/// the runtime is consulted (a settled outcome cannot be parked; the runtime
/// would only surface the same refusal as an anyhow error → 500).
pub async fn interrupt_workflow(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    match state.store.get_todo_workflow(&id).await {
        Ok(Some(record)) if record.status == "completed" || record.status == "failed" => {
            return error_409(&format!(
                "workflow 已终态（{}），不能 interrupt: {id}",
                record.status
            ));
        }
        Ok(Some(_)) => {}
        Ok(None) => return error_404(&format!("workflow 不存在: {id}")),
        Err(e) => return error_500(format!("get workflow: {e:#}")),
    }
    match opencoder_todos::interrupt(&state.store, &id, "web interrupt requested").await {
        Ok(final_state) => {
            Json(json!({ "ok": true, "status": final_state.status.as_str() })).into_response()
        }
        Err(e) => error_500(format!("interrupt: {e:#}")),
    }
}

/// POST /api/todo/workflows/:id/resume — take over a parked (suspended)
/// workflow in a spawned runtime. `running` is a 409 (two drivers would fight
/// the generation CAS — same contract as the CLI); terminal workflows are
/// answered `{ok:true, terminal:<status>}` WITHOUT spawning a runtime — the
/// explicit `terminal` marker distinguishes the no-op from a real takeover.
pub async fn resume_workflow(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let record = match state.store.get_todo_workflow(&id).await {
        Ok(Some(record)) => record,
        Ok(None) => return error_404(&format!("workflow 不存在: {id}")),
        Err(e) => return error_500(format!("get workflow: {e:#}")),
    };
    if record.status == "running" {
        return error_409("workflow 仍在运行，先 interrupt 再 resume");
    }
    if record.status == "completed" || record.status == "failed" {
        return Json(json!({ "ok": true, "terminal": record.status })).into_response();
    }
    let config = match opencoder_core::Config::load(&state.workdir) {
        Ok(config) => config,
        Err(e) => return error_500(format!("config: {e}")),
    };
    let client = match client_for(&state, &config) {
        Ok(client) => client,
        Err(resp) => return resp,
    };
    let runtime = opencoder_todos::Runtime {
        store: state.store.clone(),
        client,
        config,
        workdir: state.workdir.clone(),
        debug_root: None,
        cancel: tokio_util::sync::CancellationToken::new(),
    };
    let rid = id.clone();
    tokio::spawn(async move {
        if let Err(e) = runtime.resume(&rid).await {
            tracing::error!(workflow_id = %rid, "todo resume failed: {e:#}");
        }
    });
    Json(json!({ "ok": true })).into_response()
}
