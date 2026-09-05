//! 计划运行驱动：spawn 后台把 todo 草稿交给 plan 代理，落库方案并回写
//! todo.plan_md。结构上镜像 `crates/todos/src/execution.rs` 的直驱模式
//! （prepare_session → SessionState + cancel token → run → flusher await →
//! watermark 后取最新 assistant 文本），但产出是 markdown 方案而非候选
//! JSON。同时承载 `client_for` / `runtime_setup` / `close_run` 等被
//! execute 驱动复用的共享小件。

use std::sync::Arc;

use anyhow::{Context as _, Result};
use opencoder_core::{resolve_agent, Config, Message, Role};
use opencoder_llm::{ChatClient, ChatStream};
use opencoder_store::{
    ProjectTodoPatch, ProjectTodoRunPatch, ProjectTodoRunStatus, ProjectTodoStatus, SessionMeta,
    TASK_TYPE_PROJECT,
};
use tokio_util::sync::CancellationToken;

use crate::{context, service::Deps};

/// 客户端解析：override 优先（测试注入 MockChatClient），否则按 config
/// endpoint 构建真客户端。镜像 web `build_client`。
pub fn client_for(
    config: &Config,
    client_override: Option<Arc<dyn ChatStream>>,
) -> Result<Arc<dyn ChatStream>> {
    if let Some(client) = client_override {
        return Ok(client);
    }
    let ep = config.resolve_endpoint().context("resolve endpoint")?;
    let client = ChatClient::new_with_read_timeout(
        &ep.base_url,
        &ep.api_key,
        &ep.headers,
        config.stream_idle_timeout(),
        config.network.proxy.as_deref(),
    )
    .context("build chat client")?;
    Ok(Arc::new(client) as Arc<dyn ChatStream>)
}

/// 运行前置：加载 workdir 配置、关掉 autopilot（项目运行是显式触发，
/// 不允许代理自动续跑）、解析 LLM 客户端。
pub fn runtime_setup(deps: &Deps) -> Result<(Config, Arc<dyn ChatStream>)> {
    let mut config = Config::load(&deps.workdir).context("load config")?;
    config.autopilot.mode = opencoder_core::ApMode::Off;
    let client = client_for(&config, deps.client_override.clone())?;
    Ok((config, client))
}

/// 终结一个 run 行（状态 + 输出 + finished_at），失败只告警：run 驱动
/// 收尾路径上的写库失败不应让后台任务 panic。
pub(crate) async fn close_run(
    deps: &Deps,
    run_id: &str,
    status: ProjectTodoRunStatus,
    output: Option<String>,
    session_id: Option<String>,
) {
    let now = opencoder_core::message::now_ms();
    let patch = ProjectTodoRunPatch {
        plan_md: None,
        output_md: output,
        session_id,
        status: Some(status),
        finished_at: Some(now),
    };
    if let Err(e) = deps.projects.patch_todo_run(run_id, &patch, now).await {
        tracing::warn!(run_id, error = %e, "patch project run failed");
    }
}

/// 条件终态收敛：仅当 run 行仍处 Running 才改写，返回是否赢得 CAS。驱动
/// 可能在 close_run 与 todo 回写的两个 await 之间 panic 或被并发收敛，
/// 此时 run 已终态、输出已持久化——无条件改写只会把终态标签打花。
pub(crate) async fn close_run_if_running(
    deps: &Deps,
    run_id: &str,
    status: ProjectTodoRunStatus,
    output: Option<String>,
    session_id: Option<String>,
) -> bool {
    let now = opencoder_core::message::now_ms();
    let patch = ProjectTodoRunPatch {
        plan_md: None,
        output_md: output,
        session_id,
        status: Some(status),
        finished_at: Some(now),
    };
    match deps
        .projects
        .patch_todo_run_when(run_id, ProjectTodoRunStatus::Running, &patch, now)
        .await
    {
        Ok(won) => won,
        Err(e) => {
            tracing::warn!(run_id, error = %e, "conditional close project run failed");
            false
        }
    }
}

/// 回写 todo 状态（执行/计划收尾用），同样只告警不冒泡。
pub(crate) async fn patch_todo_status(
    deps: &Deps,
    todo_id: &str,
    status: ProjectTodoStatus,
    plan_md: Option<Option<String>>,
) {
    let now = opencoder_core::message::now_ms();
    let patch = ProjectTodoPatch {
        title: None,
        draft: None,
        plan_md,
        status: Some(status),
        agent: None,
        milestone_id: None,
        active_session_id: None,
    };
    if let Err(e) = deps.projects.patch_todo(todo_id, &patch, now).await {
        tracing::warn!(todo_id, error = %e, "patch project todo failed");
    }
}

/// 方案成功产出的 todo 回写：重读 todo，仅当其仍处非 Running 状态时按
/// 观察到的状态条件 CAS 落 Planned + plan_md。start_execute 已拒绝 plan
/// 进行中启动执行，但「检查→claim」之间有毫秒级窗口：execute 可能在
/// plan 收尾前一瞬 claim 成功（todo→Running）。无条件回写会把 Running
/// 打回 Planned，使第三次 execute 能再次 claim（双执行同一会话）；条件
/// CAS 关死该窗口——已变 Running 则丢弃 todo 回写（方案仍留痕于 run 行
/// 的 output_md）。
pub(crate) async fn commit_plan_output(deps: &Arc<Deps>, todo_id: &str, output: String) {
    let todo = match deps.projects.get_todo(todo_id).await {
        Ok(Some(todo)) => todo,
        Ok(None) | Err(_) => {
            tracing::warn!(todo_id, "plan output dropped: todo row vanished");
            return;
        }
    };
    if todo.status == ProjectTodoStatus::Running {
        tracing::warn!(
            todo_id,
            "plan output dropped: todo claimed by execute while plan was finishing"
        );
        return;
    }
    let now = opencoder_core::message::now_ms();
    let patch = ProjectTodoPatch {
        plan_md: Some(Some(output)),
        status: Some(ProjectTodoStatus::Planned),
        ..Default::default()
    };
    match deps
        .projects
        .patch_todo_when(todo_id, todo.status, &patch, now)
        .await
    {
        Ok(true) => {}
        Ok(false) => {
            tracing::warn!(todo_id, "plan writeback lost the status race; dropped");
        }
        Err(e) => {
            tracing::warn!(todo_id, error = %e, "plan output todo writeback failed");
        }
    }
}

/// 从 spawns 注册表摘除 run 令牌：drive 的最终一步（无论成败）。
pub(crate) fn forget_spawn(deps: &Deps, run_id: &str) {
    deps.spawns.lock().unwrap().remove(run_id);
}

/// watermark 之后最新一条 assistant 消息的文本；无则 None。resume 场景下
/// 旧 transcript 里的 assistant 消息不能被误认成本次产出。
pub(crate) fn latest_new_assistant(messages: &[Message], watermark: usize) -> Option<String> {
    messages
        .iter()
        .skip(watermark)
        .rev()
        .find(|m| m.role == Role::Assistant)
        .map(|m| m.text())
}

async fn create_plan_session(
    deps: &Deps,
    session_id: &str,
    todo: &opencoder_store::ProjectTodoRecord,
    config: &Config,
) -> Result<()> {
    let now = opencoder_core::message::now_ms();
    deps.store
        .create_session(&SessionMeta {
            id: session_id.into(),
            title: Some(format!("项目计划 / {}", todo.title)),
            agent: Some("plan".into()),
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
        .context("create plan session")
}

/// 计划运行主体。无论从哪条路径退出都要 forget_spawn，保证注册表不泄漏。
pub async fn drive(
    deps: Arc<Deps>,
    run_id: String,
    todo: opencoder_store::ProjectTodoRecord,
    cx: context::ProjectContext,
    cancel: CancellationToken,
) {
    if let Err(e) = run_plan(&deps, &run_id, &todo, &cx, &cancel).await {
        // 前置失败（配置/客户端/代理解析/会话落库）也必须收敛 run 行。
        tracing::warn!(run_id = %run_id, error = %e, "project plan run failed to start");
        close_run(
            &deps,
            &run_id,
            ProjectTodoRunStatus::Failed,
            Some(format!("{e:#}")),
            None,
        )
        .await;
    }
    forget_spawn(&deps, &run_id);
}

async fn run_plan(
    deps: &Arc<Deps>,
    run_id: &str,
    todo: &opencoder_store::ProjectTodoRecord,
    cx: &context::ProjectContext,
    cancel: &CancellationToken,
) -> Result<()> {
    let (config, client) = runtime_setup(deps)?;
    let agent = resolve_agent("plan").context("resolve plan agent")?;
    let session_id = ulid::Ulid::new().to_string();
    create_plan_session(deps, &session_id, todo, &config).await?;
    let mut session = opencoder_session::SessionState::new(
        session_id,
        agent,
        config,
        client,
        deps.workdir.clone(),
    )
    .with_store(deps.store.clone())
    .mark_session_created();
    session.cancel = Some(cancel.clone());
    let watermark = session.messages.len();
    let prompt = context::plan_prompt(cx);
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
        tracing::warn!(session = %session.id, error = %e, "project plan event flush failed");
    }
    finish_plan_run(deps, run_id, &todo.id, result, &session, watermark, cancel).await;
    Ok(())
}

/// run() 结果 → run 行 + todo 回写。硬取消时 session runner 返回
/// `Ok(())` 且不带新 assistant 消息（空回合被丢弃），所以 Cancelled 的
/// 判定必须在「无输出」分支里也看 cancel 令牌，不能只看 Err 路径。
async fn finish_plan_run(
    deps: &Arc<Deps>,
    run_id: &str,
    todo_id: &str,
    result: Result<()>,
    session: &opencoder_session::SessionState,
    watermark: usize,
    cancel: &CancellationToken,
) {
    match result {
        Err(_) if cancel.is_cancelled() => {
            close_run(deps, run_id, ProjectTodoRunStatus::Cancelled, None, None).await;
        }
        Err(e) => {
            close_run(
                deps,
                run_id,
                ProjectTodoRunStatus::Failed,
                Some(format!("{e:#}")),
                None,
            )
            .await;
        }
        Ok(()) => match latest_new_assistant(&session.messages, watermark) {
            Some(output) => {
                close_run(
                    deps,
                    run_id,
                    ProjectTodoRunStatus::Done,
                    Some(output.clone()),
                    Some(session.id.clone()),
                )
                .await;
                // 方案生成成功：todo 进入 Planned 并保存方案正文（条件 CAS，
                // todo 被 execute 抢先 claim 时丢弃回写）。
                commit_plan_output(deps, todo_id, output.clone()).await;
            }
            None if cancel.is_cancelled() => {
                close_run(deps, run_id, ProjectTodoRunStatus::Cancelled, None, None).await;
            }
            None => {
                close_run(
                    deps,
                    run_id,
                    ProjectTodoRunStatus::Failed,
                    Some("plan agent returned no output".into()),
                    None,
                )
                .await;
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencoder_store::{LibsqlStore, ProjectStore};
    use std::collections::HashMap;
    use std::sync::Mutex;

    // commit_plan_output 的条件回写语义：Running 让路、非 Running 落
    // Planned、行缺失静默。镜像 service.rs 测试的内存库工厂。

    async fn test_deps() -> (tempfile::TempDir, Arc<Deps>, Arc<LibsqlStore>) {
        let store = Arc::new(LibsqlStore::open_memory().await.unwrap());
        let dir = tempfile::tempdir().unwrap();
        let deps = Arc::new(Deps {
            store: store.clone(),
            projects: store.clone(),
            workdir: dir.path().to_path_buf(),
            client_override: None,
            spawns: Mutex::new(HashMap::new()),
        });
        (dir, deps, store)
    }

    async fn seed_todo(p: &Arc<LibsqlStore>, id: &str, status: ProjectTodoStatus) {
        let now = 1000;
        p.create_todo(&opencoder_store::ProjectTodoRecord {
            id: id.into(),
            milestone_id: None,
            title: format!("待办 {id}"),
            draft: "草稿".into(),
            plan_md: Some("# 旧方案".into()),
            status,
            agent: "act".into(),
            active_session_id: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn commit_plan_output_skips_running_todo() {
        let (_dir, deps, p) = test_deps().await;
        seed_todo(&p, "t1", ProjectTodoStatus::Running).await;

        commit_plan_output(&deps, "t1", "# 新方案".into()).await;

        let todo = p.get_todo("t1").await.unwrap().unwrap();
        assert_eq!(todo.status, ProjectTodoStatus::Running);
        assert_eq!(
            todo.plan_md.as_deref(),
            Some("# 旧方案"),
            "execute 已 claim（Running）时丢弃回写，不覆盖 plan_md"
        );
    }

    #[tokio::test]
    async fn commit_plan_output_writes_planned_todo() {
        let (_dir, deps, p) = test_deps().await;
        seed_todo(&p, "t1", ProjectTodoStatus::Draft).await;

        commit_plan_output(&deps, "t1", "# 新方案".into()).await;

        let todo = p.get_todo("t1").await.unwrap().unwrap();
        assert_eq!(todo.status, ProjectTodoStatus::Planned);
        assert_eq!(todo.plan_md.as_deref(), Some("# 新方案"));
    }

    #[tokio::test]
    async fn commit_plan_output_tolerates_missing_todo() {
        let (_dir, deps, _p) = test_deps().await;
        // 行缺失（被并发删除）：只告警不 panic。
        commit_plan_output(&deps, "missing", "# 新方案".into()).await;
    }
}
