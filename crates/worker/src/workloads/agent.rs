mod initialization;

use super::agent_how::{
    agent_result, declared_how_append, default_title, transcript_tail, OUTPUT_TAIL_BYTES,
};
use crate::{journal::Record, operations::native, Worker};
use anyhow::{bail, Result};
use opencoder_core::{fleet::*, Config};
use opencoder_store::SessionMeta;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

/// Optional labels a workload owner sets when it first creates a session row.
pub(crate) struct SessionLabels {
    pub title: Option<String>,
    pub kind: Option<String>,
}

pub(crate) async fn create_session(
    worker: &Worker,
    id: &str,
    agent: &str,
    model: Option<String>,
    created_at: i64,
    workdir: &std::path::Path,
    labels: SessionLabels,
) -> Result<()> {
    if worker.inner.state.store.get_session(id).await?.is_some() {
        return Ok(());
    }
    let meta = SessionMeta {
        id: id.into(),
        kind: labels.kind,
        title: labels.title,
        agent: Some(agent.into()),
        model,
        created_at,
        updated_at: created_at,
        workdir_hash: Some(opencoder_core::workdir_hash(workdir)),
        autopilot_mode: None,
        summary: None,
        summary_seq: None,
        summary_images: vec![],
        handoff_seq: None,
        handoff_plan: None,
        skill: None,
        task_type: None,
        requirement: None,
    };
    worker.inner.state.store.create_session(&meta).await
}

pub(super) async fn run(
    worker: &Worker,
    record: &Record,
    config: Config,
    cancel: CancellationToken,
    resume: bool,
) -> Result<(ExecutionStatus, Value)> {
    let assignment = &record.assignment;
    let id = &assignment.index.id;
    let _tools = (assignment.request.kind == ExecutionKind::Maintenance)
        .then(|| crate::maintenance_tools::install(worker, id));
    let input = &assignment.request.input;
    let kind = assignment.request.kind;
    let agent = assignment.request.target.as_deref().unwrap_or("act");
    // kind=agent only: the workflow-declared how.md append. Operator/
    // maintenance inputs ignore the field (legacy behavior untouched).
    let how_append = declared_how_append(kind, input)?;
    // Operator isolation: the execution's frozen (workspace, config-home)
    // pair when materialized; other kinds keep node/brain workdir resolution
    // with live config discovery.
    let (session_workdir, config_home) = crate::brain::workdir::session_dirs(worker, record)?;
    // kind=agent against a `run_mode: agent` card: every turn is one runc
    // sandbox round instead of a host session loop (see `agent_runc`).
    // Operator/maintenance never take this path — their `kind` differs even
    // when they share the host session code below.
    if kind == ExecutionKind::Agent
        && config
            .agent
            .agents_dir
            .as_deref()
            .is_some_and(|root| super::agent_runc::session_uses_sandbox(root, agent))
    {
        return super::agent_runc::run_round(worker, record, config, cancel, how_append).await;
    }
    let fresh = worker.inner.state.store.get_session(id).await?.is_none();
    let before = worker
        .inner
        .state
        .store
        .events_after(id, 0)
        .await?
        .last()
        .and_then(|e| e.seq)
        .unwrap_or(0);
    let before = record.result["monitor_after"].as_i64().unwrap_or(before);
    create_session(
        worker,
        id,
        agent,
        input["model"].as_str().map(str::to_owned),
        assignment.index.created_at,
        &session_workdir,
        SessionLabels {
            title: default_title(kind, input["title"].as_str()),
            kind: Some(kind.prefix().to_string()),
        },
    )
    .await?;
    // A session row can survive a crash before its runtime is saved.
    // Complete the durable initialization before admitting any input.
    initialization::ensure(
        worker,
        id,
        agent,
        input,
        &config,
        how_append.as_deref(),
        config_home.as_deref(),
    )
    .await?;
    let mut initial_driver_ensured = false;
    if let Some(prompt) = input["prompt"].as_str().filter(|s| !s.trim().is_empty()) {
        let prompt = match kind {
            ExecutionKind::Maintenance => format!("你是本节点的维护 agent。使用 node_maintenance 工具查询真实的状态、日志、资源和任务；只有用户明确要求时才修改配置或控制任务，不主动修复，不删除鉴权数据。\n\n用户指令：{prompt}"),
            // Operator runs the agent loop directly in the host process (no
            // runc sandbox, no node_maintenance tool), so the preamble asks
            // for host-level care instead of the maintenance tool contract.
            ExecutionKind::Operator => format!("你是 Operator agent：直接运行在宿主机进程内（非 runc 容器，也非节点维护模式）。你的操作会直接影响宿主机环境，请谨慎执行，避免破坏性与不可逆命令，仅完成用户明确交代的任务。\n\n用户指令：{prompt}"),
            _ => prompt.into(),
        };
        let reply = native(
            worker,
            "POST",
            &format!("/api/sessions/{id}/prompt"),
            json!({
                "input_id": format!("initial-{id}"),
                "prompt": prompt,
                "display": input["prompt"],
                "delivery": "queue",
                "images": input.get("images").cloned().unwrap_or(json!([])),
            }),
        )
        .await?;
        if reply.status >= 300 {
            bail!("prompt rejected: {}", reply.body);
        }
        initial_driver_ensured = reply.body["driver_ensured"]
            .as_bool()
            .ok_or_else(|| anyhow::anyhow!("prompt response missing driver_ensured"))?;
    }
    if resume
        && !fresh
        && !initial_driver_ensured
        && record.result["next_action"].as_str() == Some("resume")
    {
        opencoder_web::handle::ensure_drain(
            worker.inner.state.handles.clone(),
            worker.inner.state.store.clone(),
            id,
            worker.client(&config)?,
            session_workdir.clone(),
            config_home.clone(),
            config,
        )
        .await;
    }
    loop {
        let handle = worker.inner.state.handles.lock().await.get(id).cloned();
        if let Some(handle) = &handle {
            if cancel.is_cancelled() {
                handle.cancel.lock().await.cancel();
                opencoder_session::fire_child_cancels(&handle.child_cancels);
            }
            if handle.draining.load(std::sync::atomic::Ordering::SeqCst) {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                continue;
            }
        }
        break;
    }
    let events = worker.inner.state.store.events_after(id, before).await?;
    let last_error = events
        .iter()
        .rev()
        .find(|e| e.sse_kind.as_deref() == Some("error"));
    if let Some(error) = last_error {
        bail!("agent execution failed: {}", error.payload);
    }
    let cancelled = cancel.is_cancelled()
        || events.iter().any(|e| {
            e.sse_kind.as_deref() == Some("status") && e.payload["status"] == "interrupted"
        });
    let status = if cancelled {
        ExecutionStatus::Cancelled
    } else {
        ExecutionStatus::Idle
    };
    // Successful Agent and Operator turns expose the bounded answer for
    // linked project TODOs. Maintenance keeps the session-pointer result.
    if status == ExecutionStatus::Idle {
        if let Some(delta) = how_append.as_deref().filter(|d| !d.trim().is_empty()) {
            match opencoder_dag_runtime::exec::how_append::append_to_how_md(agent, delta) {
                Ok(version) => {
                    tracing::info!(%id, %agent, version, "how_append persisted to agent prompt pool")
                }
                Err(error) => tracing::warn!(
                    %id,
                    %agent,
                    %error,
                    "how_append persistence failed (execution outcome unchanged)"
                ),
            }
        }
        if matches!(kind, ExecutionKind::Agent | ExecutionKind::Operator) {
            // The turn's authoritative text lands via the per-turn messages
            // append, so the store projection is the transcript of record.
            let messages = worker.inner.state.store.load_messages(id).await?;
            let text =
                opencoder_session::handoff::last_assistant_text(&messages).unwrap_or_default();
            let text = transcript_tail(&text, OUTPUT_TAIL_BYTES);
            let output_json = opencoder_dag_runtime::exec::agent::extract_output_json_from(&text);
            return Ok((status, agent_result(id, &text, output_json)));
        }
    }
    Ok((status, json!({"session_id":id})))
}
