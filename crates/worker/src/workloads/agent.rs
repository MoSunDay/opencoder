use crate::{journal::Record, operations::native, Worker};
use anyhow::{bail, Result};
use opencoder_core::{fleet::*, Config};
use opencoder_store::SessionMeta;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

pub(super) async fn create_session(
    worker: &Worker,
    id: &str,
    agent: &str,
    model: Option<String>,
    title: Option<String>,
    created_at: i64,
) -> Result<()> {
    if worker.inner.state.store.get_session(id).await?.is_some() {
        return Ok(());
    }
    let meta = SessionMeta {
        id: id.into(),
        title,
        agent: Some(agent.into()),
        model,
        created_at,
        updated_at: created_at,
        workdir_hash: Some(opencoder_core::workdir_hash(&worker.inner.state.workdir)),
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
    let agent = assignment.request.target.as_deref().unwrap_or("act");
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
        input["title"].as_str().map(str::to_owned).or_else(|| {
            (assignment.request.kind == ExecutionKind::Maintenance).then(|| "节点维护".into())
        }),
        assignment.index.created_at,
    )
    .await?;
    if fresh {
        let selection = input
            .get("harness")
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()?;
        let envs = input
            .get("envs")
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()?
            .unwrap_or_default();
        opencoder_core::agent::scope::with_root(
            config.agent.agents_dir.clone(),
            opencoder_session::harness::initialize(
                worker.inner.state.store.as_ref(),
                id,
                agent,
                selection,
                envs,
            ),
        )
        .await?;
        {
            let mut runtime = worker
                .inner
                .state
                .store
                .harness_runtime(id)
                .await?
                .unwrap_or_default();
            if runtime.harness == opencoder_core::harness::Harness::Codex {
                opencoder_core::harness::pin_agent_settings(&mut runtime, &config, agent)
                    .map_err(anyhow::Error::msg)?;
                if config.agent.codex.is_none() {
                    runtime.model = input["model"].as_str().map(str::to_owned);
                }
                worker
                    .inner
                    .state
                    .store
                    .set_harness_runtime(id, &runtime)
                    .await?;
            }
        }
    }
    let mut initial_driver_ensured = false;
    if let Some(prompt) = input["prompt"].as_str().filter(|s| !s.trim().is_empty()) {
        let prompt = if assignment.request.kind == ExecutionKind::Maintenance {
            format!("你是本节点的维护 agent。使用 node_maintenance 工具查询真实的状态、日志、资源和任务；只有用户明确要求时才修改配置或控制任务，不主动修复，不删除鉴权数据。\n\n用户指令：{prompt}")
        } else {
            prompt.into()
        };
        let reply = native(
            worker,
            "POST",
            &format!("/api/sessions/{id}/prompt"),
            json!({
                "input_id": format!("initial-{id}"),
                "prompt": prompt,
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
            worker.inner.state.workdir.clone(),
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
    Ok((
        if cancel.is_cancelled()
            || events.iter().any(|e| {
                e.sse_kind.as_deref() == Some("status") && e.payload["status"] == "interrupted"
            })
        {
            ExecutionStatus::Cancelled
        } else {
            ExecutionStatus::Idle
        },
        json!({"session_id":id}),
    ))
}
