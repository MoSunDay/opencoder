//! playbook 执行器驱动：把 todo 变成一次本地剧本编排（brain 双轨调度的
//! 编排轨落地端）。`executor_ref` 指向 brain playbook 表里的一份
//! `PlaybookSpec`，驱动负责把图跑完：
//! 1. spec 来源：`store.get_brain_playbook(executor_ref)` 取已登记剧本，
//!    解析 `spec_json` 后再过一遍 `opencode_brain::playbook::validate`
//!    （纵深防御——写入口校验过，读回时也不信任存量数据；见
//!    `playbook_step::load_spec`）；
//! 2. 就绪集调度：`ready_steps` 给出依赖全完成的步骤，逐批并发执行
//!    （`JoinSet`）；任一步骤失败后 `collapse_blocked` 把全部下游步骤
//!    一次性判死（不再启动）；
//! 3. 每个步骤在「启动时」落一条 `kind=Step` 的子 run 行（普通插入、不
//!    claim），并递归经 `executor::drive` 派发到该步骤的目标执行器
//!    （agent/team/dag 直驱；brain 钉住能力经派发交接走 brain_drive；
//!    todos 工作流本地跑不了，步骤级失败——解析见 `playbook_step`）；
//! 4. 生命周期归属：子 Step 行永不回写 todo 状态（finish_todo_run 对
//!    Step 不动 todo），父 run 行（Execute）独占 todo 生命周期，在全部
//!    步骤收敛后一次性关闭；取消经 child_token 联动传播到在飞步骤。

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use opencoder_brain::playbook::{collapse_blocked, ready_steps, topo_order, PlaybookSpec};
use opencoder_store::{
    ProjectExecutorKind, ProjectTodoRecord, ProjectTodoRunKind, ProjectTodoRunRecord,
    ProjectTodoRunStatus,
};
use serde_json::json;
use tokio::task::{JoinHandle, JoinSet};
use tokio_util::sync::CancellationToken;

use crate::{
    context::ProjectContext,
    executor::{
        playbook_step::{load_spec, resolve_step, situation_for},
        BrainHandoff, ResolvedExecutor,
    },
    plan_gen::{close_run, forget_spawn},
    service::{run_agent_label, Deps},
};

/// 一个已收敛步骤的读回结果（驱动返回后重读子 run 行）。
struct StepOutcome {
    name: String,
    status: Option<ProjectTodoRunStatus>,
    output: Option<String>,
}

/// 以与其他驱动一致的 harness/agent 作用域包装 spawn 步骤驱动，返回
/// JoinHandle（JoinSet 任务内 await：panic 时当场兜底收敛，镜像
/// recover::spawn_run_driver 的监控语义）。
#[allow(clippy::too_many_arguments)]
fn spawn_step_driver(
    deps: &Arc<Deps>,
    run_id: String,
    todo: ProjectTodoRecord,
    cx: ProjectContext,
    version: i64,
    resolved: ResolvedExecutor,
    handoff: Option<BrainHandoff>,
    token: CancellationToken,
) -> JoinHandle<()> {
    let deps = deps.clone();
    tokio::spawn(opencoder_core::harness::scope::with_settings(
        opencoder_core::harness::scope::current(),
        opencoder_core::agent::scope::with_root(
            opencoder_core::agent::scope::current_root(),
            async move {
                crate::executor::drive(deps, run_id, todo, cx, version, resolved, handoff, token)
                    .await;
            },
        ),
    ))
}

/// 步骤启动：落 `kind=Step` 子 run 行（普通插入、不 claim——claim 是
/// Execute 的专利）+ 注册子取消令牌 + 派发。失败按步骤失败收敛（由调用
/// 方记账）。
#[allow(clippy::too_many_arguments)]
async fn start_step(
    deps: &Arc<Deps>,
    spec: &PlaybookSpec,
    step: &opencoder_brain::playbook::PlaybookStep,
    resolved_step: &ResolvedExecutor,
    step_todo: &ProjectTodoRecord,
    handoff: Option<BrainHandoff>,
    cx: &ProjectContext,
    version: i64,
    token: &CancellationToken,
    join: &mut JoinSet<StepOutcome>,
) -> Result<(), String> {
    let child_id = format!("prun-{}", ulid::Ulid::new());
    let now = opencoder_core::message::now_ms();
    let trace = handoff.as_ref().and_then(|h| h.trace.as_ref());
    let rec = ProjectTodoRunRecord {
        id: child_id.clone(),
        todo_id: step_todo.id.clone(),
        kind: ProjectTodoRunKind::Step,
        version,
        plan_md: None,
        output_md: None,
        agent: run_agent_label(resolved_step, step_todo),
        executor_kind: resolved_step.kind,
        capability_id: trace.and_then(|t| t.capability_id.clone()),
        plan_id: trace.and_then(|t| t.plan_id.clone()),
        output_ref: None,
        session_id: None,
        status: ProjectTodoRunStatus::Running,
        started_at: now,
        finished_at: None,
        created_at: now,
        input_snapshot: Some(
            json!({
                "schema": 1,
                "request": {"playbook": spec.id, "step": step.name},
                "todo": step_todo,
                "context": cx,
                "executor": {"kind": resolved_step.kind, "ref": resolved_step.ref_},
            })
            .to_string(),
        ),
        trace_manifest: None,
    };
    deps.projects
        .create_todo_run(&rec)
        .await
        .map_err(|e| format!("create step run for {:?}: {e:#}", step.name))?;
    let child_token = token.child_token();
    deps.spawns
        .lock()
        .unwrap()
        .insert(child_id.clone(), child_token.clone());
    // brain 步骤经交接走 brain_drive（重解析采纳 override），其余直驱已
    // 解析的具体执行器——递归深度按构造为 1（BrainRouteKind 只有
    // agent/team/dag，没有 playbook/brain 嵌套）。
    let drive_resolved = if handoff.is_some() {
        ResolvedExecutor {
            kind: ProjectExecutorKind::Brain,
            ref_: None,
        }
    } else {
        resolved_step.clone()
    };
    let driver = spawn_step_driver(
        deps,
        child_id.clone(),
        step_todo.clone(),
        cx.clone(),
        version,
        drive_resolved,
        handoff,
        child_token,
    );
    let deps = deps.clone();
    let todo_id = step_todo.id.clone();
    let name = step.name.clone();
    join.spawn(async move {
        let panicked = matches!(driver.await, Err(e) if e.is_panic());
        if panicked {
            tracing::error!(run_id = %child_id, step = %name, "playbook step driver panicked; converging");
            crate::recover::converge_panicked_run(
                &deps,
                &child_id,
                &todo_id,
                ProjectTodoRunKind::Step,
            )
            .await;
        }
        forget_spawn(&deps, &child_id);
        let row = deps.projects.get_todo_run(&child_id).await.ok().flatten();
        StepOutcome {
            name,
            status: row.as_ref().map(|r| r.status),
            output: row.and_then(|r| r.output_md),
        }
    });
    Ok(())
}

/// 一轮调度状态（就绪集循环的可变载体）：done/failed/cancelled 与逐步骤
/// 输出记账（outputs 是 done/failed 的超集，兼作「已收敛步骤」去重集）。
#[derive(Default)]
struct Schedule {
    done: BTreeSet<String>,
    failed: BTreeSet<String>,
    cancelled: bool,
    outputs: BTreeMap<String, (ProjectTodoRunStatus, Option<String>)>,
}

impl Schedule {
    /// 记账一个收敛步骤：Done → done；Cancelled → cancelled + 取消父令牌
    /// （一个步骤被取消即终止整本剧本）；其余（失败/读不回行）→ failed。
    fn settle(&mut self, out: StepOutcome, token: &CancellationToken) {
        let StepOutcome {
            name,
            status,
            output,
        } = out;
        match status {
            Some(ProjectTodoRunStatus::Done) => {
                self.done.insert(name.clone());
                self.outputs
                    .insert(name, (ProjectTodoRunStatus::Done, output));
            }
            Some(ProjectTodoRunStatus::Cancelled) => {
                self.cancelled = true;
                token.cancel();
                self.outputs
                    .insert(name, (ProjectTodoRunStatus::Cancelled, output));
            }
            other => {
                self.failed.insert(name.clone());
                self.outputs.insert(
                    name,
                    (other.unwrap_or(ProjectTodoRunStatus::Failed), output),
                );
            }
        }
    }

    /// 排空在飞步骤（父取消路径的收尾）。
    async fn drain(&mut self, join: &mut JoinSet<StepOutcome>, token: &CancellationToken) {
        while let Some(out) = join.join_next().await {
            self.settle(
                out.expect("playbook step task must not be cancelled"),
                token,
            );
        }
    }
}

/// 汇总输出：按拓扑序每步一行 `- {name}: {status}[: {excerpt ≤200 字符}]`。
fn summarize(
    spec: &PlaybookSpec,
    outputs: &BTreeMap<String, (ProjectTodoRunStatus, Option<String>)>,
) -> String {
    let order =
        topo_order(spec).unwrap_or_else(|_| spec.steps.iter().map(|s| s.name.clone()).collect());
    let mut lines = vec!["剧本步骤结果：".to_string()];
    for name in order {
        if let Some((status, output)) = outputs.get(&name) {
            let mut line = format!("- {}: {}", name, status.as_str());
            if let Some(text) = output.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
                let excerpt: String = text.chars().take(200).collect();
                line.push_str(&format!(": {excerpt}"));
            }
            lines.push(line);
        }
    }
    lines.join("\n")
}

/// 就绪集调度主体：返回父 run 的终态 + 汇总输出（由调用方一次性关闭父
/// run 行——循环结束前绝不 close）。
async fn run_playbook(
    deps: &Arc<Deps>,
    run_id: &str,
    todo: &ProjectTodoRecord,
    cx: &ProjectContext,
    version: i64,
    resolved: &ResolvedExecutor,
    token: &CancellationToken,
) -> (ProjectTodoRunStatus, String) {
    let ref_ = resolved
        .ref_
        .as_deref()
        .or(todo.executor_ref.as_deref())
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .unwrap_or_default();
    let spec = match load_spec(deps, ref_).await {
        Ok(spec) => spec,
        Err(msg) => {
            tracing::warn!(run_id = run_id, error = %msg, "project playbook load failed");
            return (ProjectTodoRunStatus::Failed, msg);
        }
    };
    let situation = situation_for(cx, todo);
    let mut s = Schedule::default();
    let mut join: JoinSet<StepOutcome> = JoinSet::new();
    loop {
        if token.is_cancelled() {
            // child_token 已联动取消在飞步骤；排空 join 后统一收尾。
            s.cancelled = true;
            break;
        }
        let blocked = collapse_blocked(&spec, &s.failed);
        for name in ready_steps(&spec, &s.done) {
            if s.outputs.contains_key(&name) || blocked.contains(&name) {
                continue;
            }
            let Some(step) = spec.steps.iter().find(|st| st.name == name) else {
                continue;
            };
            let started = match resolve_step(deps, step, todo, cx, &situation).await {
                Err(msg) => Err(msg),
                Ok((resolved_step, step_todo, handoff)) => {
                    start_step(
                        deps,
                        &spec,
                        step,
                        &resolved_step,
                        &step_todo,
                        handoff,
                        cx,
                        version,
                        token,
                        &mut join,
                    )
                    .await
                }
            };
            if let Err(msg) = started {
                tracing::warn!(run_id = run_id, step = %name, error = %msg, "playbook step failed to start");
                s.failed.insert(name.clone());
                s.outputs
                    .insert(name, (ProjectTodoRunStatus::Failed, Some(msg)));
            }
        }
        // 守卫：本批无可调度且无在飞任务 → 结束（全部完成或全被阻断）。
        if join.is_empty() {
            break;
        }
        while let Some(out) = join.join_next().await {
            s.settle(
                out.expect("playbook step task must not be cancelled"),
                token,
            );
        }
        if s.cancelled {
            break;
        }
    }
    s.drain(&mut join, token).await;
    let status = if s.cancelled {
        ProjectTodoRunStatus::Cancelled
    } else if !s.failed.is_empty() {
        ProjectTodoRunStatus::Failed
    } else if s.done.len() == spec.steps.len() {
        ProjectTodoRunStatus::Done
    } else {
        // 无失败却未全部完成（理论不可达）：保守按失败收敛，不悬 Running。
        ProjectTodoRunStatus::Failed
    };
    (status, summarize(&spec, &s.outputs))
}

/// playbook 执行器入口：编排整本剧本后一次性关闭父 run 行（Execute 行，
/// 独占 todo 状态回写）并摘除注册——任何退出路径都会走到这里。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn drive(
    deps: Arc<Deps>,
    run_id: String,
    todo: ProjectTodoRecord,
    cx: ProjectContext,
    version: i64,
    resolved: ResolvedExecutor,
    token: CancellationToken,
) {
    let (status, output) =
        run_playbook(&deps, &run_id, &todo, &cx, version, &resolved, &token).await;
    close_run(&deps, &run_id, status, Some(output), None, None).await;
    forget_spawn(&deps, &run_id);
}
