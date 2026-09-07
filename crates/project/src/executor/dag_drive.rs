//! dag 执行器驱动：把 todo 变成一次本地工作流执行。复用
//! `opencoder-dag-runtime`（不经过节点注册/平台派发）：
//! 1. spec 来源：内联 `executor_spec`（解析 + `opencoder_dag::validate`）
//!    优先，否则 `store.get_dag_def(executor_ref)` 取已登记定义；
//! 2. dag run id 直接用项目 run id（`prun-…` 满足 validate_run_id），
//!    并以该 id 建宿主 session（agent "act"），让 DAG 事件/会话列表可见；
//! 3. `Uplink::for_local_dag` + `LocalDagEvents` 把事件批量落到
//!    `append_events`（kind=Step，sse_kind 透传），status 报告仅接受；
//! 4. 取消经 watch channel 桥接到 `execute_run`；终态映射 Done/Cancelled/
//!    失败，output_ref = 工件根目录，run 行 session_id = 宿主 session。

use std::path::PathBuf;

use anyhow::{anyhow, Context as _, Result};
use async_trait::async_trait;
use opencoder_dag::{
    decode_spec_str, validate as validate_dag, DagClaimedRun, DagEventBatch, DagSpec,
    DagStatusReport,
};
use opencoder_node::uplink::{LocalDagPersistence, Uplink};
use opencoder_store::{
    EventKind, ProjectTodoRecord, ProjectTodoRunStatus, ProjectTodoStatus, SessionEventRecord,
    SessionMeta, Store, TASK_TYPE_PROJECT,
};
use serde_json::json;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use crate::{
    context::ProjectContext,
    executor::ResolvedExecutor,
    plan_gen::{close_run, forget_spawn, patch_todo_status, runtime_setup},
    service::Deps,
};

/// 本地 DAG 事件落库（镜像 worker `workloads::dag::LocalEvents`，去掉
/// failure 记账——这里 append 失败直接作为 uplink 错误反馈给运行时）。
struct LocalDagEvents {
    store: Arc<dyn Store>,
}

#[async_trait]
impl LocalDagPersistence for LocalDagEvents {
    async fn events(&self, batch: &DagEventBatch) -> Result<()> {
        let rows: Vec<SessionEventRecord> = batch
            .events
            .iter()
            .map(|e| SessionEventRecord {
                session_id: batch.run_id.clone(),
                kind: EventKind::Step,
                payload: json!({"kind":e.kind,"step":e.step,"payload":e.payload,"at_ms":e.at_ms}),
                ts: e.at_ms,
                seq: None,
                sse_kind: Some(e.kind.clone()),
            })
            .collect();
        self.store.append_events(&rows).await.map(|_| ())
    }

    async fn status(&self, _report: &DagStatusReport) -> Result<()> {
        Ok(())
    }
}

/// 解析 dag spec：内联优先，其次按 executor_ref 取已登记定义。返回
/// (spec, dag_id)：内联时 dag_id 取 spec.name，引用时取定义登记名。
async fn resolve_spec(
    deps: &Arc<Deps>,
    todo: &ProjectTodoRecord,
    resolved: &ResolvedExecutor,
) -> Result<(DagSpec, String)> {
    if let Some(spec_json) = todo.executor_spec.as_deref() {
        let spec: DagSpec =
            decode_spec_str(spec_json).map_err(|e| anyhow!("parse dag executor spec: {e}"))?;
        validate_dag(&spec).map_err(|e| anyhow!("invalid dag spec: {}", e.join("; ")))?;
        let dag_id = spec.name.clone();
        return Ok((spec, dag_id));
    }
    let ref_ = resolved
        .ref_
        .as_deref()
        .or(todo.executor_ref.as_deref())
        .ok_or_else(|| anyhow!("dag executor requires executor_ref or executor_spec"))?;
    let def = deps
        .store
        .get_dag_def(ref_)
        .await
        .with_context(|| format!("load dag definition {ref_:?}"))?
        .ok_or_else(|| anyhow!("dag definition not found: {ref_}"))?;
    let spec: DagSpec = decode_spec_str(&def.spec_json)
        .map_err(|e| anyhow!("parse dag definition {ref_:?} spec: {e}"))?;
    validate_dag(&spec)
        .map_err(|e| anyhow!("dag definition {ref_:?} invalid: {}", e.join("; ")))?;
    Ok((spec, def.name))
}

/// 宿主 session：id 即 dag run id（DAG 事件以 run_id 为 session_id 落
/// 库，挂上会话列表），agent "act"、任务类型 project。
async fn create_host_session(
    deps: &Arc<Deps>,
    run_id: &str,
    todo: &ProjectTodoRecord,
    config: &opencoder_core::Config,
) -> Result<()> {
    let now = opencoder_core::message::now_ms();
    let excerpt: String = todo
        .plan_md
        .as_deref()
        .or(Some(todo.draft.as_str()))
        .unwrap_or("")
        .chars()
        .take(200)
        .collect();
    deps.store
        .create_session(&SessionMeta {
            id: run_id.to_string(),
            title: Some(format!("项目DAG / {}", todo.title)),
            agent: Some("act".into()),
            model: Some(config.model.clone()),
            autopilot_mode: None,
            workdir_hash: None,
            created_at: now,
            updated_at: now,
            summary: None,
            summary_seq: None,
            summary_images: Vec::new(),
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: Some(TASK_TYPE_PROJECT.into()),
            requirement: Some(excerpt),
        })
        .await
        .context("create dag host session")
}

/// 步落摘要：每个 step 读 `<run>/<step>/meta.json` 的 outcome 组行，
/// 附带最多 8 个存在的工件文件路径（output.txt / output.json）。step
/// 目录一律经 `artifacts::step_dir`（slug 校验，拒绝路径穿越）。
fn collect_output_md(workflow_root: &std::path::Path, run_id: &str, spec: &DagSpec) -> String {
    let mut lines = Vec::new();
    let mut artifacts = Vec::new();
    for step in &spec.steps {
        let dir = match opencoder_dag::artifacts::step_dir(workflow_root, run_id, &step.name) {
            Ok(dir) => dir,
            Err(_) => continue,
        };
        let outcome = std::fs::read_to_string(dir.join("meta.json"))
            .ok()
            .and_then(|body| serde_json::from_str::<serde_json::Value>(&body).ok())
            .and_then(|v| v.get("outcome").and_then(|o| o.as_str()).map(Into::into))
            .unwrap_or_else(|| "unknown".to_string());
        lines.push(format!("- {}: {}", step.name, outcome));
        for file in ["output.txt", "output.json"] {
            if artifacts.len() < 8 && dir.join(file).exists() {
                artifacts.push(dir.join(file).display().to_string());
            }
        }
    }
    let mut out = String::from("DAG 步骤结果：\n");
    out.push_str(&lines.join("\n"));
    if !artifacts.is_empty() {
        out.push_str("\n\n工件：\n");
        out.push_str(&artifacts.join("\n"));
    }
    out
}

/// dag 执行器入口。任何失败路径都收敛 run 行 + todo 状态并摘除注册。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn drive(
    deps: Arc<Deps>,
    run_id: String,
    todo: ProjectTodoRecord,
    cx: ProjectContext,
    _version: i64,
    resolved: ResolvedExecutor,
    token: CancellationToken,
) {
    let _ = cx;
    let result = run_dag(&deps, &run_id, &todo, &resolved, &token).await;
    let (status, todo_status, output, output_ref, session_id) = match result {
        Ok((status, run_root, workflow_root, spec)) => {
            let output = collect_output_md(&workflow_root, &run_id, &spec);
            let closed = Some(run_id.clone());
            match status {
                opencoder_dag::DagRunStatus::Done => (
                    ProjectTodoRunStatus::Done,
                    ProjectTodoStatus::Done,
                    Some(output),
                    Some(run_root.display().to_string()),
                    closed,
                ),
                opencoder_dag::DagRunStatus::Cancelled => (
                    ProjectTodoRunStatus::Cancelled,
                    ProjectTodoStatus::Planned,
                    Some(output),
                    Some(run_root.display().to_string()),
                    closed,
                ),
                other => (
                    ProjectTodoRunStatus::Failed,
                    ProjectTodoStatus::Failed,
                    Some(format!("DAG 终态 {}：\n{output}", other.as_str())),
                    Some(run_root.display().to_string()),
                    closed,
                ),
            }
        }
        // 宿主 session 已创建时 close 仍写 session_id 供会话列表回看；
        // 未创建时不得留悬挂引用（session_id 留空）。
        Err(f) => (
            ProjectTodoRunStatus::Failed,
            ProjectTodoStatus::Failed,
            Some(format!("{:#}", f.error)),
            None,
            f.host_session.then(|| run_id.clone()),
        ),
    };
    close_run(&deps, &run_id, status, output, output_ref, session_id).await;
    patch_todo_status(&deps, &todo.id, todo_status, None).await;
    forget_spawn(&deps, &run_id);
}

/// run_dag 失败载荷：错误 + 宿主 session 是否已创建（已创建时 close 仍
/// 写 session_id 供会话列表回看；未创建时不得留悬挂引用）。
struct DagFailure {
    error: anyhow::Error,
    host_session: bool,
}

/// 宿主 session 创建之前的步骤失败（运行时装配 / spec 解析 / 工件根 /
/// 建宿主会话本身）：会话未落库，失败载荷不带 host_session 标记。
fn pre_session_failure(error: anyhow::Error) -> DagFailure {
    DagFailure {
        error,
        host_session: false,
    }
}

async fn run_dag(
    deps: &Arc<Deps>,
    run_id: &str,
    todo: &ProjectTodoRecord,
    resolved: &ResolvedExecutor,
    token: &CancellationToken,
) -> Result<(opencoder_dag::DagRunStatus, PathBuf, PathBuf, DagSpec), DagFailure> {
    let (config, client) = runtime_setup(deps).map_err(pre_session_failure)?;
    let (spec, dag_id) = resolve_spec(deps, todo, resolved)
        .await
        .map_err(pre_session_failure)?;
    let workflow_root = opencoder_core::data_dir_for(&deps.workdir).join("workflow");
    let run_root = opencoder_dag::artifacts::run_root(&workflow_root, run_id)
        .map_err(|e| pre_session_failure(anyhow!("illegal dag run id {run_id:?}: {e}")))?;
    create_host_session(deps, run_id, todo, &config)
        .await
        .map_err(pre_session_failure)?;

    let uplink = Arc::new(Uplink::for_local_dag(Arc::new(LocalDagEvents {
        store: deps.store.clone(),
    })));
    let run_deps = opencoder_dag_runtime::RunDeps {
        uplink,
        exec: opencoder_dag_runtime::ExecDeps {
            store: deps.store.clone(),
            client,
            workdir: deps.workdir.clone(),
            config,
        },
        workflow_root: workflow_root.clone(),
    };
    // 取消桥：项目 run 的 CancellationToken → watch<bool>（forwarder 在
    // execute_run 返回后中止，避免泄漏）。
    let (tx, rx) = tokio::sync::watch::channel(false);
    let fwd = {
        let ct = token.clone();
        tokio::spawn(async move {
            ct.cancelled().await;
            let _ = tx.send(true);
        })
    };
    let claimed = DagClaimedRun {
        run_id: run_id.to_string(),
        dag_id,
        spec: spec.clone(),
        created_at: opencoder_core::message::now_ms(),
    };
    let status = opencoder_dag_runtime::execute_run(run_deps, claimed, rx).await;
    fwd.abort();
    // 宿主 session 已创建：此后失败 close 仍写 session_id 供回看。
    let status = status.map_err(|e| DagFailure {
        error: e,
        host_session: true,
    })?;
    Ok((status, run_root, workflow_root, spec))
}
