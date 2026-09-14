//! 执行器维度：把 todo 的执行从「写死的 act 会话直驱」泛化为四类执行器
//! —— agent（既有会话直驱，`agent_drive`）、team（本地多人讨论，
//! `team_drive`）、dag（本地工作流，`dag_drive`）、brain（能力库路由后
//! 递归派发，`brain_drive`）。本模块承载纯解析（`resolve` 把 todo 的
//! 执行器三字段收敛成一个目标、`retarget` 把目标回写进 todo 克隆）与
//! 唯一派发入口 `drive`；spec 类型与校验在 `spec` 子模块。所有子驱动
//! 保持同一 run 行生命周期（claim → drive → close_run → todo 回写）、
//! 同一取消注册表与 panic/stale 兜底（recover.rs），差异只在「谁干活」。

mod agent_drive;
mod brain_drive;
mod dag_drive;
mod playbook_drive;
mod playbook_step;
mod team_drive;

use std::sync::Arc;

use anyhow::{bail, Result};
use tokio_util::sync::CancellationToken;

use opencoder_store::{ProjectExecutorKind, ProjectTodoRecord};

use crate::{context::ProjectContext, service::Deps};

pub use brain_drive::{resolve_brain, BrainTrace};
/// 内联 spec 的类型与校验是纯域逻辑，住在 `opencoder-store`（见其模块
/// 文档：控制面共享的 API 文件不能链接本 crate 的执行引擎）；此处按
/// P1 的路径 `executor::spec::*` 再导出，消费方（team/brain 驱动与
/// executor 集成测试）保持不变。
pub use opencoder_store::project_executor_spec as spec;
pub use spec::validate_spec;

/// 解析后的执行目标。brain 永远不是 RESOLVED kind：`resolve` 拒绝
/// brain→brain 嵌套，brain todo 由 `brain_drive` 解析成 agent/team/dag
/// 之一后才进入派发。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedExecutor {
    pub kind: ProjectExecutorKind,
    pub ref_: Option<String>,
}

/// brain todo 派发交接：控制面 override（claim 前解析已采纳，驱动内
/// 重解析直接复用该纯函数分支——节点无 brain 运行时也可执行）与
/// claim 前解析留痕。
#[derive(Debug, Clone, Default)]
pub struct BrainHandoff {
    pub override_: Option<crate::service::ExecutorOverride>,
    pub trace: Option<BrainTrace>,
}

/// 解析 todo 的执行目标（纯函数）。override（控制面预解析）优先；否则
/// 按 todo.executor_kind：Agent 一律直驱；Team/Dag 要求 executor_ref 或
/// executor_spec 至少其一；Brain 在此报错——本地 brain 解析（可能触发
/// LLM 路由调用）由 `brain_drive::resolve_brain` 承担，调用方在 claim
/// 之前先走它。
pub fn resolve(
    todo: &ProjectTodoRecord,
    override_: Option<&crate::service::ExecutorOverride>,
) -> Result<ResolvedExecutor> {
    if let Some(ov) = override_ {
        if ov.kind == ProjectExecutorKind::Brain {
            bail!("executor override cannot pre-resolve the brain executor");
        }
        return Ok(ResolvedExecutor {
            kind: ov.kind,
            ref_: ov.ref_.clone(),
        });
    }
    match todo.executor_kind {
        ProjectExecutorKind::Agent => Ok(ResolvedExecutor {
            kind: ProjectExecutorKind::Agent,
            ref_: None,
        }),
        ProjectExecutorKind::Team => with_target(todo, ProjectExecutorKind::Team),
        ProjectExecutorKind::Dag => with_target(todo, ProjectExecutorKind::Dag),
        ProjectExecutorKind::Playbook => with_target(todo, ProjectExecutorKind::Playbook),
        ProjectExecutorKind::Brain => Err(anyhow::anyhow!(
            "brain executor requires runtime resolution"
        )),
    }
}

fn with_target(todo: &ProjectTodoRecord, kind: ProjectExecutorKind) -> Result<ResolvedExecutor> {
    let ref_ = todo.executor_ref.clone().filter(|r| !r.trim().is_empty());
    if ref_.is_none() && todo.executor_spec.is_none() {
        bail!(
            "{} executor requires executor_ref or executor_spec",
            kind.as_str()
        );
    }
    Ok(ResolvedExecutor { kind, ref_ })
}

/// 把解析出的目标回写进 todo 克隆（纯函数）：agent 目标携带代理名时覆盖
/// todo.agent（brain 递归派发需要）；team/dag 目标携带 ref 时覆盖
/// executor_ref（executor_spec 原样保留——内联 spec 优先于 ref）。
pub(crate) fn retarget(todo: &ProjectTodoRecord, resolved: &ResolvedExecutor) -> ProjectTodoRecord {
    let mut next = todo.clone();
    match resolved.kind {
        ProjectExecutorKind::Agent => {
            if let Some(name) = resolved.ref_.as_deref() {
                next.agent = name.to_string();
            }
        }
        ProjectExecutorKind::Team | ProjectExecutorKind::Dag => {
            if let Some(name) = resolved.ref_.as_deref() {
                next.executor_ref = Some(name.to_string());
            }
        }
        // playbook 的目标就是 todo 自身的 executor_ref（驱动内再展开成
        // 步骤图），无额外回写；brain 同理。
        ProjectExecutorKind::Brain | ProjectExecutorKind::Playbook => {}
    }
    next
}

/// 派发入口：按解析结果分派到对应子驱动。`brain` 是 brain todo 的派发
/// 交接（控制面 override 随行，驱动内重解析直接采纳；claim 前解析留痕
/// 单一事实源），run 行的 capability/plan 已由 service 在 claim 前打点，
/// 各子驱动的 close 不触碰这些字段（patch 的 None = 保持原值）。
#[allow(clippy::too_many_arguments)]
pub async fn drive(
    deps: Arc<Deps>,
    run_id: String,
    todo: ProjectTodoRecord,
    cx: ProjectContext,
    version: i64,
    resolved: ResolvedExecutor,
    brain: Option<BrainHandoff>,
    token: CancellationToken,
) {
    let todo = retarget(&todo, &resolved);
    match resolved.kind {
        ProjectExecutorKind::Agent => {
            agent_drive::drive(deps, run_id, todo, cx, version, token).await
        }
        ProjectExecutorKind::Team => {
            team_drive::drive(deps, run_id, todo, cx, version, resolved, token).await
        }
        ProjectExecutorKind::Dag => {
            dag_drive::drive(deps, run_id, todo, cx, version, resolved, token).await
        }
        ProjectExecutorKind::Brain => {
            brain_drive::drive(deps, run_id, todo, cx, version, brain, token).await
        }
        ProjectExecutorKind::Playbook => {
            playbook_drive::drive(deps, run_id, todo, cx, version, resolved, token).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::ExecutorOverride;

    fn todo(
        kind: ProjectExecutorKind,
        ref_: Option<&str>,
        spec: Option<&str>,
    ) -> ProjectTodoRecord {
        ProjectTodoRecord {
            id: "t1".into(),
            milestone_id: None,
            title: "待办".into(),
            draft: "草稿".into(),
            plan_md: Some("# 方案".into()),
            status: opencoder_store::ProjectTodoStatus::Planned,
            agent: "act".into(),
            executor_kind: kind,
            executor_ref: ref_.map(Into::into),
            executor_spec: spec.map(Into::into),
            active_session_id: None,
            created_at: 1,
            updated_at: 1,
        }
    }

    /// agent 缺省：无视 ref/spec 一律直驱 act。
    #[test]
    fn resolve_agent_default_ignores_extras() {
        let r = resolve(
            &todo(ProjectExecutorKind::Agent, Some("x"), Some("{}")),
            None,
        )
        .unwrap();
        assert_eq!(r.kind, ProjectExecutorKind::Agent);
        assert_eq!(r.ref_, None);
    }

    /// team 带 ref：目标 = 团队名；spec-only 时 ref 为 None 但不报错。
    #[test]
    fn resolve_team_with_ref_or_spec() {
        let r = resolve(&todo(ProjectExecutorKind::Team, Some("fleet"), None), None).unwrap();
        assert_eq!(r.kind, ProjectExecutorKind::Team);
        assert_eq!(r.ref_.as_deref(), Some("fleet"));
        let r = resolve(&todo(ProjectExecutorKind::Team, None, Some("{}")), None).unwrap();
        assert_eq!(r.ref_, None);
    }

    /// team/dag ref 与 spec 双缺 → 明确报错。
    #[test]
    fn resolve_team_or_dag_missing_both_errors() {
        let e = resolve(&todo(ProjectExecutorKind::Team, None, None), None).unwrap_err();
        assert!(e.to_string().contains("team executor requires"), "{e}");
        let e = resolve(&todo(ProjectExecutorKind::Dag, None, None), None).unwrap_err();
        assert!(e.to_string().contains("dag executor requires"), "{e}");
        // 空白 ref 视同缺失。
        assert!(resolve(&todo(ProjectExecutorKind::Dag, Some("  "), None), None).is_err());
    }

    /// brain 无 override → 报「需运行时解析」；override 优先且禁止 brain。
    #[test]
    fn resolve_brain_errors_without_runtime_resolution() {
        let e = resolve(&todo(ProjectExecutorKind::Brain, None, None), None).unwrap_err();
        assert!(
            e.to_string().contains("brain executor requires runtime"),
            "{e}"
        );
    }

    #[test]
    fn resolve_override_wins_over_todo_fields() {
        let ov = ExecutorOverride {
            kind: ProjectExecutorKind::Dag,
            ref_: Some("dag-def".into()),
            capability_id: Some("cap-1".into()),
            plan_id: None,
        };
        // 覆盖一个 agent todo：按 override 走 dag。
        let r = resolve(&todo(ProjectExecutorKind::Agent, None, None), Some(&ov)).unwrap();
        assert_eq!(r.kind, ProjectExecutorKind::Dag);
        assert_eq!(r.ref_.as_deref(), Some("dag-def"));
        // override 禁止 brain（brain 不能被预解析成 brain）。
        let bad = ExecutorOverride {
            kind: ProjectExecutorKind::Brain,
            ref_: None,
            capability_id: None,
            plan_id: None,
        };
        assert!(resolve(&todo(ProjectExecutorKind::Brain, None, None), Some(&bad)).is_err());
    }

    /// retarget：agent 带名覆盖 todo.agent；team/dag 带 ref 覆盖
    /// executor_ref 并保留 executor_spec；无 ref 时原样。
    #[test]
    fn retarget_threads_resolved_target_into_todo() {
        let base = todo(ProjectExecutorKind::Brain, Some("cap-1"), Some("{}"));
        let agent = retarget(
            &base,
            &ResolvedExecutor {
                kind: ProjectExecutorKind::Agent,
                ref_: Some("explore".into()),
            },
        );
        assert_eq!(agent.agent, "explore");
        let dag = retarget(
            &base,
            &ResolvedExecutor {
                kind: ProjectExecutorKind::Dag,
                ref_: Some("dag-def".into()),
            },
        );
        assert_eq!(dag.executor_ref.as_deref(), Some("dag-def"));
        assert_eq!(dag.executor_spec.as_deref(), Some("{}"), "spec preserved");
        let untouched = retarget(
            &base,
            &ResolvedExecutor {
                kind: ProjectExecutorKind::Agent,
                ref_: None,
            },
        );
        assert_eq!(untouched.agent, "act");
        assert_eq!(untouched.executor_ref.as_deref(), Some("cap-1"));
    }
}
