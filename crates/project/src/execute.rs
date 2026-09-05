//! 执行运行驱动：把 todo 的现行方案交给主代理在工作目录中落地。
//! 「新或续」会话策略：todo.active_session_id 存在且会话仍可加载时
//! resume 同一 session（持续推进，上下文跨执行延续），否则新建 session
//! 并把 id 回写到 todo.active_session_id。结构与 plan_gen 相同（直驱
//! SessionState + run + flusher），复用其 runtime_setup/close_run 小件。

use std::sync::Arc;

use anyhow::{Context as _, Result};
use opencoder_llm::ChatStream;
use opencoder_store::{
    ProjectTodoPatch, ProjectTodoRecord, ProjectTodoRunStatus, ProjectTodoStatus, SessionMeta,
    TASK_TYPE_PROJECT,
};
use tokio_util::sync::CancellationToken;

use crate::{
    context,
    plan_gen::{close_run, forget_spawn, latest_new_assistant, patch_todo_status, runtime_setup},
    service::Deps,
};

async fn create_execute_session(
    deps: &Deps,
    session_id: &str,
    todo: &ProjectTodoRecord,
    config: &opencoder_core::Config,
) -> Result<()> {
    let now = opencoder_core::message::now_ms();
    deps.store
        .create_session(&SessionMeta {
            id: session_id.into(),
            title: Some(format!("项目执行 / {}", todo.title)),
            agent: Some(todo.agent.clone()),
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
            requirement: Some(todo.draft.clone()),
        })
        .await
        .context("create execute session")
}

/// 新或续：todo.active_session_id 指向的会话仍存在 → resume（返回
/// `resumed = true`）；否则新建 session 并回写 active_session_id。
async fn new_or_resume_session(
    deps: &Arc<Deps>,
    todo: &ProjectTodoRecord,
    config: &opencoder_core::Config,
    client: Arc<dyn ChatStream>,
) -> Result<(opencoder_session::SessionState, bool)> {
    if let Some(sid) = todo.active_session_id.as_deref() {
        let existing = deps.store.get_session(sid).await.context("load session")?;
        if existing.is_some() {
            let session = opencoder_session::resume(
                deps.store.clone(),
                sid,
                config.clone(),
                client,
                deps.workdir.clone(),
            )
            .await
            .with_context(|| format!("resume session {sid}"))?;
            return Ok((session, true));
        }
    }
    let agent = opencoder_core::resolve_agent(&todo.agent)
        .with_context(|| format!("todo {} has unknown agent {}", todo.id, todo.agent))?;
    let session_id = ulid::Ulid::new().to_string();
    create_execute_session(deps, &session_id, todo, config).await?;
    let session = opencoder_session::SessionState::new(
        session_id,
        agent,
        config.clone(),
        client,
        deps.workdir.clone(),
    )
    .with_store(deps.store.clone())
    .mark_session_created();
    let now = opencoder_core::message::now_ms();
    let patch = ProjectTodoPatch {
        title: None,
        draft: None,
        plan_md: None,
        status: None,
        agent: None,
        milestone_id: None,
        active_session_id: Some(Some(session.id.clone())),
    };
    deps.projects
        .patch_todo(&todo.id, &patch, now)
        .await
        .context("record active session")?;
    Ok((session, false))
}

/// 执行运行主体。任何失败路径都要把 run 行与 todo 状态一并收敛（todo
/// 由 start_execute 置为 Running，不能悬在 Running 上），并在最后摘除
/// spawn 注册。
pub async fn drive(
    deps: Arc<Deps>,
    run_id: String,
    todo: ProjectTodoRecord,
    cx: context::ProjectContext,
    version: i64,
    cancel: CancellationToken,
) {
    if let Err(e) = run_execute(&deps, &run_id, &todo, &cx, version, &cancel).await {
        tracing::warn!(run_id = %run_id, error = %e, "project execute run failed");
        close_run(
            &deps,
            &run_id,
            ProjectTodoRunStatus::Failed,
            Some(format!("{e:#}")),
            None,
        )
        .await;
        patch_todo_status(&deps, &todo.id, ProjectTodoStatus::Failed, None).await;
    }
    forget_spawn(&deps, &run_id);
}

async fn run_execute(
    deps: &Arc<Deps>,
    run_id: &str,
    todo: &ProjectTodoRecord,
    cx: &context::ProjectContext,
    version: i64,
    cancel: &CancellationToken,
) -> Result<()> {
    let (config, client) = runtime_setup(deps)?;
    let (mut session, resumed) = new_or_resume_session(deps, todo, &config, client).await?;
    session.cancel = Some(cancel.clone());
    let watermark = session.messages.len();
    let plan_md = todo.plan_md.as_deref().unwrap_or("");
    let prompt = context::execute_prompt(cx, plan_md, version, resumed);
    let (sink, flusher) =
        opencoder_session::spawn_event_flusher(Some(deps.store.clone()), session.id.clone());
    let result = opencoder_session::run(&mut session, prompt, {
        let sink = sink.clone();
        move |ev| {
            let _ = sink.push(&ev);
        }
    })
    .await;
    drop(sink);
    if let Err(e) = flusher.await {
        tracing::warn!(session = %session.id, error = %e, "project execute event flush failed");
    }
    finish_execute_run(deps, run_id, &todo.id, result, &session, watermark, cancel).await;
    Ok(())
}

/// 结果收敛：Done/Failed/Cancelled 三态。硬取消时 session runner 返回
/// `Ok(())` 且不落任何新 assistant 消息，所以 Cancelled 判定同时覆盖
/// Err 路径与「Ok 但无输出」路径；取消把 todo 回退到 Planned（方案仍在，
/// 用户可再次执行）。
async fn finish_execute_run(
    deps: &Arc<Deps>,
    run_id: &str,
    todo_id: &str,
    result: Result<()>,
    session: &opencoder_session::SessionState,
    watermark: usize,
    cancel: &CancellationToken,
) {
    let session_id = Some(session.id.clone());
    match result {
        Ok(()) => match latest_new_assistant(&session.messages, watermark) {
            Some(output) => {
                close_run(
                    deps,
                    run_id,
                    ProjectTodoRunStatus::Done,
                    Some(output),
                    session_id,
                )
                .await;
                patch_todo_status(deps, todo_id, ProjectTodoStatus::Done, None).await;
            }
            None if cancel.is_cancelled() => {
                close_run(
                    deps,
                    run_id,
                    ProjectTodoRunStatus::Cancelled,
                    None,
                    session_id,
                )
                .await;
                patch_todo_status(deps, todo_id, ProjectTodoStatus::Planned, None).await;
            }
            None => {
                close_run(
                    deps,
                    run_id,
                    ProjectTodoRunStatus::Failed,
                    Some("execute agent returned no output".into()),
                    session_id,
                )
                .await;
                patch_todo_status(deps, todo_id, ProjectTodoStatus::Failed, None).await;
            }
        },
        Err(_) if cancel.is_cancelled() => {
            close_run(
                deps,
                run_id,
                ProjectTodoRunStatus::Cancelled,
                None,
                session_id,
            )
            .await;
            patch_todo_status(deps, todo_id, ProjectTodoStatus::Planned, None).await;
        }
        Err(e) => {
            close_run(
                deps,
                run_id,
                ProjectTodoRunStatus::Failed,
                Some(format!("{e:#}")),
                session_id,
            )
            .await;
            patch_todo_status(deps, todo_id, ProjectTodoStatus::Failed, None).await;
        }
    }
}
