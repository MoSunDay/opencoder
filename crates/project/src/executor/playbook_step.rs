//! playbook 步骤解析（纯映射为主）：situation 文本构造、剧本 spec 读回
//! 复核、步骤 → 执行目标的解析。与调度循环（`playbook_drive`）按功能边界
//! 拆开——本模块回答「这一步交给谁、带着什么指令」，不碰任何 run 行。

use std::sync::Arc;

use opencoder_brain::playbook::{
    render_prompt, validate, PlaybookRouteKind, PlaybookSpec, PlaybookStep, PlaybookTarget,
};
use opencoder_store::{ProjectExecutorKind, ProjectTodoRecord};

use crate::{
    context::ProjectContext,
    executor::{BrainHandoff, BrainTrace, ResolvedExecutor},
    service::{Deps, ExecutorOverride},
};

/// situation 文本（`{situation}` 占位符的替换值）：目标/里程碑/待办标题 +
/// 草稿链路，两侧 trim，截到 2000 字符（步骤模板只吃得到这些上下文）。
pub(crate) fn situation_for(cx: &ProjectContext, todo: &ProjectTodoRecord) -> String {
    let mut s = String::new();
    if let Some(title) = &cx.goal_title {
        s.push_str(&format!("目标：{title}\n"));
    }
    if let Some(m) = &cx.milestone_title {
        s.push_str(&format!("里程碑：{m}\n"));
    }
    s.push_str(&format!("待办：{}\n", todo.title));
    s.push_str(&format!("草稿：{}", todo.draft.trim()));
    s.chars().take(2000).collect()
}

/// 读取并复核剧本 spec。任何失败都以 `String` 返回（驱动层把它作为父
/// run 的失败输出收敛），消息与 dag_drive 的「引用不存在/数据损坏」风格
/// 对齐。
pub(crate) async fn load_spec(deps: &Arc<Deps>, ref_: &str) -> Result<PlaybookSpec, String> {
    let record = deps
        .store
        .get_brain_playbook(ref_)
        .await
        .map_err(|e| format!("load playbook {ref_:?}: {e:#}"))?
        .ok_or_else(|| format!("playbook not found: {ref_}"))?;
    let spec: PlaybookSpec = serde_json::from_str(&record.spec_json)
        .map_err(|e| format!("stored playbook {ref_} spec is corrupt: {e}"))?;
    validate(&spec)
        .map_err(|errs| format!("stored playbook {ref_} invalid: {}", errs.join("; ")))?;
    Ok(spec)
}

/// 把解析出的目标回写进步骤 todo 克隆（供 `start_step` 落子 run 行与
/// input_snapshot 留痕）。
fn with_target(
    step_todo: &ProjectTodoRecord,
    kind: ProjectExecutorKind,
    ref_: &str,
) -> (ResolvedExecutor, ProjectTodoRecord) {
    let mut next = step_todo.clone();
    next.executor_kind = kind;
    next.executor_ref = Some(ref_.to_string());
    (
        ResolvedExecutor {
            kind,
            ref_: Some(ref_.to_string()),
        },
        next,
    )
}

/// 步骤 → 执行目标解析（在步骤启动时执行一次）。返回（解析目标、步骤
/// todo 克隆、brain 派发交接）。步骤 todo 的改动：`executor_spec` 清空
/// （父 todo 的 spec 是 playbook 引用，不是步骤执行器的 spec）；渲染后
/// 的步骤指令同时写进 `plan_md` 与 `draft`——agent 驱动的提示词吃
/// `todo.plan_md`（`context::execute_prompt`），draft 供会话 requirement
/// 留痕；`active_session_id` 清空：并发步骤不得共享/续跑同一会话。
pub(crate) async fn resolve_step(
    deps: &Arc<Deps>,
    step: &PlaybookStep,
    todo: &ProjectTodoRecord,
    cx: &ProjectContext,
    situation: &str,
) -> Result<(ResolvedExecutor, ProjectTodoRecord, Option<BrainHandoff>), String> {
    let prompt = render_prompt(&step.prompt, situation);
    let mut step_todo = todo.clone();
    step_todo.executor_spec = None;
    step_todo.plan_md = Some(prompt.clone());
    step_todo.draft = prompt;
    step_todo.active_session_id = None;
    match &step.target {
        PlaybookTarget::Agent { agent } => {
            step_todo.executor_kind = ProjectExecutorKind::Agent;
            step_todo.executor_ref = None;
            Ok((
                ResolvedExecutor {
                    kind: ProjectExecutorKind::Agent,
                    ref_: Some(agent.clone()),
                },
                step_todo,
                None,
            ))
        }
        PlaybookTarget::Team { team } => {
            let (resolved, next) = with_target(&step_todo, ProjectExecutorKind::Team, team);
            Ok((resolved, next, None))
        }
        PlaybookTarget::Dag { dag } => {
            let (resolved, next) = with_target(&step_todo, ProjectExecutorKind::Dag, dag);
            Ok((resolved, next, None))
        }
        // todos 工作流是平台语义（节点/控制面派发），本地执行引擎跑不了：
        // 步骤启动时即失败（Err 原样进入父 run 的失败汇总）。
        PlaybookTarget::Todos { .. } => {
            Err("todos target requires platform dispatch (control)".to_string())
        }
        PlaybookTarget::Brain {
            capability_id,
            route,
        } => match route {
            // 内联路由是跨端确定性通道——本地与控制面对同一 spec 解析出
            // 相同执行器，不再依赖环境绑定：不经 resolve_brain（无路由表
            // /运行时查询），路由即执行器，能力 id 仅作溯源随行。
            Some(route) => {
                let kind = match route.kind {
                    PlaybookRouteKind::Agent => ProjectExecutorKind::Agent,
                    PlaybookRouteKind::Team => ProjectExecutorKind::Team,
                    PlaybookRouteKind::Dag => ProjectExecutorKind::Dag,
                };
                let (resolved, step_todo) = with_target(&step_todo, kind, &route.ref_);
                // 交接镜像钉住能力路径的构造：路由结果包成 override 随行，
                // 驱动内重解析直接采纳（节点无 brain 运行时亦可执行）。
                let handoff = BrainHandoff {
                    override_: Some(ExecutorOverride {
                        kind: resolved.kind,
                        ref_: resolved.ref_.clone(),
                        capability_id: Some(capability_id.clone()),
                        plan_id: None,
                    }),
                    trace: Some(BrainTrace {
                        capability_id: Some(capability_id.clone()),
                        plan_id: None,
                    }),
                };
                Ok((resolved, step_todo, Some(handoff)))
            }
            None => {
                let (_, step_todo) =
                    with_target(&step_todo, ProjectExecutorKind::Brain, capability_id);
                // 钉住能力 + 缺省路由表 → 纯解析，节点无需 brain 运行时。
                let (resolved, trace) = crate::executor::resolve_brain(deps, &step_todo, cx, None)
                    .await
                    .map_err(|e| format!("brain step {:?} resolve failed: {e:#}", step.name))?;
                // 派发交接镜像 reserve_attempt/drive_reserved 对 brain todo 的
                // 构造：解析结果包成 override 随行，驱动内重解析直接采纳。
                let handoff = BrainHandoff {
                    override_: Some(ExecutorOverride {
                        kind: resolved.kind,
                        ref_: resolved.ref_.clone(),
                        capability_id: trace.capability_id.clone(),
                        plan_id: trace.plan_id.clone(),
                    }),
                    trace: Some(trace),
                };
                Ok((resolved, step_todo, Some(handoff)))
            }
        },
    }
}
