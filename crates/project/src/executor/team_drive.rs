//! team 执行器驱动：把 todo 变成一次本地团队讨论。不经过平台节点注册
//! （`start_topic` 的注册检查是平台语义），而是：
//! 1. 从内联 `TeamSpec` 或 fleet 团队（`load_team`）物化一个项目专属团队
//!    （名字按 todo id 净化，落 team_root）；
//! 2. `init_topic` 写入话题（requirement = 方案 + 目标链上下文）；
//! 3. `run_topic` 驱动讨论，成员提问由 `LocalTeamDispatcher` 用「每次
//!    ask 一个独立本地 session」回答（替代节点任务，不写 team_topic_runs
//!    台账）；
//! 4. 按 FINISH_* 映射 run/todo 终态，output_ref = topic id。

use std::sync::Arc;

use anyhow::{anyhow, Context as _, Result};
use async_trait::async_trait;
use opencoder_llm::ChatStream;
use opencoder_store::{
    ProjectExecutorKind, ProjectTodoRecord, ProjectTodoRunStatus, ProjectTodoStatus, SessionMeta,
    TASK_TYPE_PROJECT,
};
use opencoder_team::types::{MemberRef, TeamMember, TeamMeta, TopicMeta};
use opencoder_team::{
    fs_store, layout::validate_team_name, CancelToken, TeamDispatcher, TeamRunConfig,
    FINISH_CANCELLED, FINISH_COMPLETE, FINISH_ERROR, FINISH_MAX_SUB_TURNS, FINISH_MAX_TURNS,
};
use tokio_util::sync::CancellationToken;

use crate::{
    context::ProjectContext,
    executor::{spec::TeamSpec, ResolvedExecutor},
    plan_gen::{close_run, forget_spawn, latest_new_assistant, runtime_setup},
    service::Deps,
};

/// 每次成员提问直驱一个本地 session（镜像 plan_gen::run_plan：cancel 接
/// 线、watermark、flusher），返回最新 assistant 文本。node_id 按代理名
/// 解析（`resolve_agent`），解析失败即错。
struct LocalTeamDispatcher {
    deps: Arc<Deps>,
    config: opencoder_core::Config,
    client: Arc<dyn ChatStream>,
    title: String,
    cancel: CancellationToken,
}

#[async_trait]
impl TeamDispatcher for LocalTeamDispatcher {
    async fn ask(&self, _topic: Option<&str>, node_id: &str, prompt: &str) -> Result<String> {
        let agent = opencoder_core::resolve_agent(node_id)
            .with_context(|| format!("team member {node_id} is not a resolvable agent"))?;
        let session_id = ulid::Ulid::new().to_string();
        let now = opencoder_core::message::now_ms();
        let excerpt: String = prompt.chars().take(200).collect();
        self.deps
            .store
            .create_session(&SessionMeta {
                id: session_id.clone(),
                title: Some(format!("项目团队 / {}", self.title)),
                agent: Some(node_id.to_string()),
                model: Some(self.config.model.clone()),
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
            .context("create team member session")?;
        let mut session = opencoder_session::SessionState::new(
            session_id,
            agent,
            self.config.clone(),
            self.client.clone(),
            self.deps.workdir.clone(),
        )
        .with_store(self.deps.store.clone())
        .mark_session_created();
        session.cancel = Some(self.cancel.clone());
        let watermark = session.messages.len();
        let (sink, flusher) = opencoder_session::spawn_event_flusher(
            Some(self.deps.store.clone()),
            session.id.clone(),
        );
        let result = opencoder_session::run(&mut session, prompt.to_string(), {
            let sink = sink.clone();
            move |ev| {
                let _ = sink.push(&ev);
            }
        })
        .await;
        drop(sink);
        if let Err(e) = flusher.await {
            tracing::warn!(session = %session.id, error = %e, "team member event flush failed");
        }
        match result {
            Ok(()) => latest_new_assistant(&session.messages, watermark)
                .ok_or_else(|| anyhow!("team member {node_id} returned no output")),
            Err(e) => Err(e),
        }
    }
}

/// 物化团队名：`project-{净化 todo id}`。净化 = 小写、非 `[a-z0-9-]` 变
/// '-'；总长截到 ≤64；`project-` 前缀保证以字母开头。构造结果必须满足
/// `validate_team_name`。
fn materialized_team_name(todo_id: &str) -> String {
    let sanitized: String = todo_id
        .to_ascii_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() {
                c
            } else {
                '-'
            }
        })
        .collect();
    let mut name: String = format!("project-{sanitized}");
    name.truncate(64);
    debug_assert!(validate_team_name(&name), "materialized name {name:?}");
    name
}

/// 话题 requirement：目标→里程碑→待办上下文（对齐 execute_prompt 的背景
/// 段）+ 方案正文（缺失时回退草稿）。
fn team_requirement(cx: &ProjectContext, todo: &ProjectTodoRecord) -> String {
    let mut out = String::new();
    out.push_str("请就下面的项目待办展开团队讨论并收敛出结论。\n\n背景：\n");
    if let Some(title) = &cx.goal_title {
        out.push_str(&format!("- 目标：{title}\n"));
    }
    if let Some(title) = &cx.milestone_title {
        out.push_str(&format!("- 里程碑：{}\n", title));
    }
    out.push_str(&format!("- 待办：{}\n", cx.todo_title));
    let body = todo
        .plan_md
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| todo.draft.trim());
    out.push_str(&format!("\n实施方案：\n{body}\n"));
    out.push_str("\n完成标准：讨论收敛出明确结论，最终总结给出可执行的答复。\n");
    out
}

/// team_root 缺省回填规则（镜像 web `team_state::run_config_from`）：配置
/// 未显式设置（等于 Config::default 的缺省值）时挂到本 workdir 数据目录
/// 的 `team/` 下，与 store 数据同树。
fn team_root_for(config: &opencoder_core::Config, workdir: &std::path::Path) -> std::path::PathBuf {
    if config.team_root == opencoder_core::Config::default().team_root {
        opencoder_core::data_dir_for(workdir).join("team")
    } else {
        config.team_root.clone()
    }
}

fn meta_from_spec(spec: &TeamSpec, name: &str, now: i64) -> TeamMeta {
    TeamMeta {
        name: name.to_string(),
        captain: MemberRef {
            node_id: spec.captain.node_id.clone(),
            name: spec.captain.name.clone(),
        },
        members: spec
            .members
            .iter()
            .map(|m| TeamMember {
                node_id: m.node_id.clone(),
                name: m.name.clone(),
                capabilities: m.capabilities.clone(),
                profiled_at: None,
            })
            .collect(),
        created_at: now,
        updated_at: now,
    }
}

/// 无总结时的收尾摘要（run 行 output_md 不能空着）。
fn topic_digest(meta: &TopicMeta) -> String {
    format!(
        "team topic finished ({}) after {} turn(s)",
        meta.finish_reason.as_deref().unwrap_or("unknown"),
        meta.turns.len()
    )
}

/// FINISH_* → (run 终态, todo 终态, 输出)。complete 带总结 → 双 Done；
/// cancelled → run Cancelled、todo 回 Planned（方案仍在，可重试）；
/// 上限类带总结视为收敛完成，无总结判失败；error 同失败（错误文本入
/// output）。
fn map_finish(meta: &TopicMeta) -> (ProjectTodoRunStatus, ProjectTodoStatus, Option<String>) {
    let summary_nonempty = meta
        .final_summary
        .as_deref()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    match meta.finish_reason.as_deref() {
        Some(FINISH_COMPLETE) if summary_nonempty => (
            ProjectTodoRunStatus::Done,
            ProjectTodoStatus::Done,
            meta.final_summary.clone(),
        ),
        Some(FINISH_CANCELLED) => (
            ProjectTodoRunStatus::Cancelled,
            ProjectTodoStatus::Planned,
            None,
        ),
        Some(FINISH_MAX_TURNS) | Some(FINISH_MAX_SUB_TURNS) if summary_nonempty => (
            ProjectTodoRunStatus::Done,
            ProjectTodoStatus::Done,
            meta.final_summary.clone(),
        ),
        reason => (
            ProjectTodoRunStatus::Failed,
            ProjectTodoStatus::Failed,
            Some(format!(
                "team topic failed: {}",
                reason.unwrap_or(FINISH_ERROR)
            )),
        ),
    }
}

/// team 执行器入口。任何失败路径都收敛 run 行 + todo 状态并摘除注册。
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
    match run_topic_for_todo(&deps, &todo, &cx, &resolved, &token).await {
        Ok(meta) => {
            let (status, _todo_status, output) = map_finish(&meta);
            let output = output.or_else(|| Some(topic_digest(&meta)));
            close_run(
                &deps,
                &run_id,
                status,
                output,
                Some(meta.topic_id.clone()),
                None,
            )
            .await;
        }
        Err(e) => {
            tracing::warn!(run_id = %run_id, error = %e, "project team run failed");
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
    }
    forget_spawn(&deps, &run_id);
}

async fn run_topic_for_todo(
    deps: &Arc<Deps>,
    todo: &ProjectTodoRecord,
    cx: &ProjectContext,
    resolved: &ResolvedExecutor,
    token: &CancellationToken,
) -> Result<TopicMeta> {
    let (config, client) = runtime_setup(deps)?;
    let team_root = team_root_for(&config, &deps.workdir);
    let cfg = TeamRunConfig {
        team_root: team_root.clone(),
        max_turns: config.team_max_turns,
        max_sub_turns: config.team_max_sub_turns,
    };
    let name = materialized_team_name(&todo.id);
    let now = opencoder_core::message::now_ms();

    // 团队来源：内联 spec 优先，其次 fleet 团队（必须已存在）。
    let mut meta = match todo.executor_spec.as_deref() {
        Some(spec_json) => {
            let spec: TeamSpec =
                serde_json::from_str(spec_json).context("parse team executor spec")?;
            crate::executor::spec::validate_spec(ProjectExecutorKind::Team, spec_json)
                .context("validate team executor spec")?;
            meta_from_spec(&spec, &name, now)
        }
        None => {
            let team_name = resolved
                .ref_
                .as_deref()
                .or(todo.executor_ref.as_deref())
                .ok_or_else(|| anyhow!("team executor requires executor_ref or executor_spec"))?;
            let mut meta = fs_store::load_team(&team_root, team_name)
                .with_context(|| format!("load fleet team {team_name:?}"))?;
            meta.updated_at = now;
            meta
        }
    };
    meta.name = name.clone();
    fs_store::save_team(&team_root, &meta).context("save materialized team")?;

    let requirement = team_requirement(cx, todo);
    let topic = fs_store::init_topic(
        &team_root,
        &name,
        &todo.title,
        &requirement,
        meta.captain.clone(),
        meta.members
            .iter()
            .map(|m| MemberRef {
                node_id: m.node_id.clone(),
                name: m.name.clone(),
            })
            .collect(),
        now,
    )
    .context("init team topic")?;

    // 取消桥：项目 run 的 CancellationToken → 团队协作令牌（watcher 在
    // run_topic 返回后中止，避免泄漏）。
    let team_cancel = CancelToken::new();
    let watcher = {
        let flag = team_cancel.clone();
        let ct = token.clone();
        tokio::spawn(async move {
            ct.cancelled().await;
            flag.cancel();
        })
    };
    let dispatcher = Arc::new(LocalTeamDispatcher {
        deps: deps.clone(),
        config,
        client,
        title: todo.title.clone(),
        cancel: token.clone(),
    });
    let result = opencoder_team::runtime::run_topic(
        deps.store.clone(),
        dispatcher,
        &cfg,
        &name,
        &topic.topic_id,
        team_cancel,
    )
    .await;
    watcher.abort();
    result
}
