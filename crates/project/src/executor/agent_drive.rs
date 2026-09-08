//! agent 执行器驱动（自 `execute.rs` 原样迁入）：把 todo 的现行方案交给
//! 主代理在工作目录中落地。「新或续」会话策略：todo.active_session_id
//! 存在且会话仍可加载时 resume 同一 session（持续推进，上下文跨执行
//! 延续），否则新建 session 并把 id 回写到 todo.active_session_id。
//! 结构与 plan_gen 相同（直驱 SessionState + run + flusher），复用其
//! runtime_setup/close_run 小件。

use std::sync::Arc;

use anyhow::{Context as _, Result};
use opencoder_llm::ChatStream;
use opencoder_store::{
    ProjectTodoPatch, ProjectTodoRecord, ProjectTodoRunStatus, SessionMeta, TASK_TYPE_PROJECT,
};
use tokio_util::sync::CancellationToken;

use crate::{
    context,
    plan_gen::{close_run, forget_spawn, latest_attempt_assistant, runtime_setup},
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
        let identity = crate::trace::resources::identity(
            &opencoder_core::resolve_agent(&todo.agent).context("assigned agent unavailable")?,
        )?;
        let previous = deps
            .projects
            .list_todo_runs(&todo.id)
            .await?
            .into_iter()
            .find(|r| r.session_id.as_deref() == Some(sid));
        let same = previous
            .as_ref()
            .and_then(|run| run.input_snapshot.as_deref())
            .map(serde_json::from_str::<serde_json::Value>)
            .transpose()?
            .is_some_and(|input| input["agent"]["digest"] == identity["digest"]);
        if same
            && existing
                .as_ref()
                .is_some_and(|meta| meta.agent.as_deref() == Some(todo.agent.as_str()))
        {
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
        executor_kind: None,
        executor_ref: None,
        executor_spec: None,
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
pub(crate) async fn drive(
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
            None,
        )
        .await;
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
    let watermark = session
        .messages
        .iter()
        .map(|message| message.id.clone())
        .collect::<std::collections::HashSet<_>>();
    let plan_md = todo.plan_md.as_deref().unwrap_or("");
    let mut prompt = context::execute_prompt(cx, plan_md, version, resumed);
    if !resumed {
        if let Some(previous) = deps
            .projects
            .list_todo_runs(&todo.id)
            .await?
            .into_iter()
            .find(|r| {
                r.id != run_id
                    && r.kind == opencoder_store::ProjectTodoRunKind::Execute
                    && r.output_md.is_some()
            })
        {
            prompt.push_str(&format!(
                "\n上次运行 {} 的结果（接续任务的参考）：\n{}\n",
                previous.id,
                previous.output_md.as_deref().unwrap_or("")
            ));
        }
    }
    prompt.push_str("\n请用 project_artifact 工具登记需要交付的报告、补丁或其他文件，以保留本次运行的独立副本。\n");
    let trace = crate::trace::RunTrace::begin(deps, run_id, &mut session, &prompt, cancel.clone())
        .await
        .inspect_err(|error| {
            *deps.persistence_error.lock().unwrap() =
                Some(format!("project trace initialization: {error:#}"));
        })?;
    let _tools = trace.tools();
    let (sink, flusher) =
        opencoder_session::spawn_checked_event_flusher(deps.store.clone(), session.id.clone());
    let result = opencoder_session::run(&mut session, prompt, |ev| {
        trace.record(&ev);
        if let Err(error) = sink.push(&ev) {
            trace.archive.fail(error);
        }
    })
    .await;
    drop(sink);
    let flushed = flusher.await.context("project event flusher stopped")?;
    if let Err(error) = &flushed {
        trace.archive.fail(error);
    }
    trace.finish(deps).await.inspect_err(|error| {
        *deps.persistence_error.lock().unwrap() = Some(format!("project persistence: {error:#}"));
    })?;
    let result = result.and(flushed);
    finish_execute_run(deps, run_id, &todo.id, result, &session, watermark, cancel).await;
    Ok(())
}

/// Cancellation takes precedence over output from earlier tool steps.
async fn finish_execute_run(
    deps: &Arc<Deps>,
    run_id: &str,
    _todo_id: &str,
    result: Result<()>,
    session: &opencoder_session::SessionState,
    watermark: std::collections::HashSet<String>,
    cancel: &CancellationToken,
) {
    let (status, output) = crate::plan_gen::attempt_outcome(
        result,
        latest_attempt_assistant(&session.messages, &watermark),
        cancel.is_cancelled(),
        "execute agent returned no output",
    );
    close_run(deps, run_id, status, output, None, Some(session.id.clone())).await;
}
