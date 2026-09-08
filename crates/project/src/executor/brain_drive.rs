//! brain 执行器驱动：先「解析」再「递归派发」。
//!
//! 解析（`resolve_brain`，在 claim 之前由 service 调用）三级：
//! 1. 控制面 override（平台已预解析）直接采纳，能力/计划留痕取 override
//!    （kind=brain 在此被拒——禁止 brain→brain 嵌套，镜像 `executor::resolve`）；
//! 2. 钉住（todo.executor_ref = capability id）→ 免 brain 运行时：按
//!    路由表（todo.executor_spec，缺省 {agent, "act"}）pick 出目标；
//! 3. 未钉住 → 必须有 brain 运行时（节点上没有，控制面预解析）：以
//!    目标链 + 标题 + 草稿 + 方案构造 situation，`dispatch_or_plan`（top_k
//!    8、优先复用 digest 缓存计划）拿 capability + plan 留痕，再 pick。
//!
//! 派发（`drive`）：携带派发交接（`BrainHandoff`：控制面 override 随行
//! ——驱动内重解析直接采纳该纯函数分支，节点无 brain 运行时也可执行；
//! claim 前解析留痕作为单一事实源）；解析出的目标若非 agent，todo 的
//! executor_spec（那是路由表，不是目标执行器的 spec）清空、executor_ref
//! 换成路由 ref，然后递归进 `executor::drive` 复用同一 run 行；解析失败
//! 按失败收敛。

use std::sync::Arc;

use anyhow::{anyhow, bail, Context as _, Result};
use opencoder_store::{ProjectExecutorKind, ProjectTodoRecord, ProjectTodoRunStatus};
use tokio_util::sync::CancellationToken;

use crate::{
    context::ProjectContext,
    executor::{spec::BrainRoutes, BrainHandoff, ResolvedExecutor},
    plan_gen::forget_spawn,
    service::{Deps, ExecutorOverride},
};

/// brain 留痕：run 行的 capability_id / plan_id（patch 的 None = 保持
/// 原值，所以子驱动的 close 不会抹掉它）。
#[derive(Debug, Clone, Default)]
pub struct BrainTrace {
    pub capability_id: Option<String>,
    pub plan_id: Option<String>,
}

/// situation 文本：能力路由的判别输入（嵌入 + 决策树行走都吃它）。
fn situation_for(cx: &ProjectContext, todo: &ProjectTodoRecord) -> String {
    let mut s = String::new();
    s.push_str(&format!("目标：{}\n", cx.goal_title));
    if let Some(m) = &cx.milestone_title {
        s.push_str(&format!("里程碑：{m}\n"));
    }
    s.push_str(&format!("待办：{}\n", todo.title));
    s.push_str(&format!("草稿：{}\n", todo.draft.trim()));
    if let Some(p) = todo
        .plan_md
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        s.push_str(&format!("方案：{p}\n"));
    }
    s
}

/// 从 todo.executor_spec 解析路由表；缺失走缺省（{agent, "act"}）。
fn routes_for(todo: &ProjectTodoRecord) -> Result<BrainRoutes> {
    match todo.executor_spec.as_deref() {
        None => Ok(BrainRoutes {
            routes: Vec::new(),
            default: Default::default(),
        }),
        Some(json) => {
            serde_json::from_str(json).context("parse brain executor routes (executor_spec)")
        }
    }
}

/// 路由 → 解析目标。team/dag 路由必须带非空 ref（路由表带不了内联
/// spec），否则在 claim 之前就报错。
fn route_to_executor(routes: &BrainRoutes, capability: &str) -> Result<ResolvedExecutor> {
    let route = routes.pick(capability);
    match route.kind {
        crate::executor::spec::BrainRouteKind::Agent => Ok(ResolvedExecutor {
            kind: ProjectExecutorKind::Agent,
            ref_: route.ref_.clone(),
        }),
        kind @ (crate::executor::spec::BrainRouteKind::Team
        | crate::executor::spec::BrainRouteKind::Dag) => {
            let ref_ = route
                .ref_
                .as_deref()
                .map(str::trim)
                .filter(|r| !r.is_empty())
                .ok_or_else(|| {
                    anyhow!(
                        "brain route to {} requires a ref (routes carry no inline spec)",
                        ProjectExecutorKind::from(kind).as_str()
                    )
                })?;
            Ok(ResolvedExecutor {
                kind: ProjectExecutorKind::from(kind),
                ref_: Some(ref_.to_string()),
            })
        }
    }
}

/// 解析 brain todo（见模块文档三级规则）。任何错误都在 claim 之前发生，
/// service 直接向上抛，不留半启动状态。
pub async fn resolve_brain(
    deps: &Arc<Deps>,
    todo: &ProjectTodoRecord,
    cx: &ProjectContext,
    override_: Option<&ExecutorOverride>,
) -> Result<(ResolvedExecutor, BrainTrace)> {
    if let Some(ov) = override_ {
        // 禁止 brain→brain 嵌套（镜像 `executor::resolve` 的同名守卫）。
        if ov.kind == ProjectExecutorKind::Brain {
            bail!("executor override cannot pre-resolve the brain executor");
        }
        return Ok((
            ResolvedExecutor {
                kind: ov.kind,
                ref_: ov.ref_.clone(),
            },
            BrainTrace {
                capability_id: ov.capability_id.clone(),
                plan_id: ov.plan_id.clone(),
            },
        ));
    }
    let pinned = todo
        .executor_ref
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty());
    let routes = routes_for(todo)?;
    match pinned {
        Some(capability) => {
            let resolved = route_to_executor(&routes, capability)?;
            Ok((
                resolved,
                BrainTrace {
                    capability_id: Some(capability.to_string()),
                    plan_id: None,
                },
            ))
        }
        None => {
            let runtime = deps.brain.as_ref().ok_or_else(|| {
                anyhow!(
                    "brain executor requires the brain runtime \
                     (unavailable on nodes; pre-resolve on the control plane)"
                )
            })?;
            let situation = situation_for(cx, todo);
            let now = opencoder_core::message::now_ms();
            let dispatched = runtime
                .dispatch_or_plan(runtime.chat_model(), &situation, 8, false, now)
                .await
                .context("brain dispatch_or_plan failed")?;
            if dispatched.planned_fresh {
                tracing::info!(plan_id = %dispatched.record.id, "brain planned fresh decision tree");
            }
            let capability = dispatched.outcome.capability_id.clone();
            let resolved = route_to_executor(&routes, &capability)?;
            Ok((
                resolved,
                BrainTrace {
                    capability_id: Some(capability),
                    plan_id: Some(dispatched.record.id),
                },
            ))
        }
    }
}

/// brain 执行器入口：解析 → 递归派发（同一 run 行）。`handoff` 携带
/// 控制面 override（重解析直接采纳，节点无 brain 运行时也可执行）与
/// claim 前留痕。解析失败按失败收敛 run/todo。`_version` 由递归下游忽略
/// （版本号只做行版本，不再回写 todo）。递归（drive → executor::drive
/// → drive）经 Box 显式装箱以满足异步递归。
#[allow(clippy::too_many_arguments)]
pub(crate) fn drive(
    deps: Arc<Deps>,
    run_id: String,
    todo: ProjectTodoRecord,
    cx: ProjectContext,
    version: i64,
    handoff: Option<BrainHandoff>,
    token: CancellationToken,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    Box::pin(async move {
        let handoff = handoff.unwrap_or_default();
        let resolved = match resolve_brain(&deps, &todo, &cx, handoff.override_.as_ref()).await {
            Ok((resolved, trace)) => {
                // override 随行时留痕在 claim 前 INSERT 已随 run 行落库
                // （单一事实源），此处跳过同值幂等 patch；无 override 的
                // 本机 brain 路径仍按驱动内重解析结果刷新（patch 的
                // None = 保持原值，子驱动 close 不触碰这两个字段）。
                if handoff.override_.is_none()
                    && (trace.capability_id.is_some() || trace.plan_id.is_some())
                {
                    let patch = opencoder_store::ProjectTodoRunPatch {
                        capability_id: trace.capability_id,
                        plan_id: trace.plan_id,
                        ..Default::default()
                    };
                    if let Err(e) = deps
                        .projects
                        .patch_todo_run(&run_id, &patch, opencoder_core::message::now_ms())
                        .await
                    {
                        tracing::warn!(run_id = %run_id, error = %e, "stamp brain trace failed");
                    }
                }
                resolved
            }
            Err(e) => {
                tracing::warn!(run_id = %run_id, error = %e, "project brain resolution failed");
                crate::plan_gen::close_run(
                    &deps,
                    &run_id,
                    ProjectTodoRunStatus::Failed,
                    Some(format!("brain 解析失败：{e:#}")),
                    None,
                    None,
                )
                .await;

                forget_spawn(&deps, &run_id);
                return;
            }
        };
        // 路由表不是目标执行器的 spec：清空 executor_spec、ref 换成路由 ref
        // （agent 路由带名时由 retarget 覆盖 todo.agent），再递归派发。递归
        // 已是具体 kind（brain 被解析消掉），无需再交接 brain 上下文。
        let mut next = todo;
        next.executor_spec = None;
        next.executor_ref = resolved.ref_.clone();
        crate::executor::drive(deps, run_id, next, cx, version, resolved, None, token).await;
    })
}
