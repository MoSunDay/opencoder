//! Run 驱动的崩溃兜底：panic 监控收敛 + 机会式 stale run 清扫。
//!
//! 正常路径的收敛（run 终态 + todo 回写 + 摘除注册）由 `plan_gen::drive` /
//! `execute::drive` 自己负责；本模块只覆盖它们「没有机会收敛」的两种情形：
//! 驱动任务 panic（进程还活着，run 行悬在 running）与进程重启丢掉驱动
//! （注册表是内存态，重启即空）。前者由 [`spawn_run_driver`] 的监控任务
//! 当场收敛，后者由 [`sweep_stale_runs`] 在 `overview()` 读路径机会式清扫
//! ——镜像 `converge_lost_node_tasks` 的思路：无后台定时器，读一次扫一次。
//! 所有收敛写库均为条件 CAS（仅 Running 才改写）：run 已被驱动自身收敛
//! 到终态时不再打花标签，但 todo 仍 Running 时必须补收敛，否则悬死。

use std::{future::Future, sync::Arc};

use opencoder_store::{
    ProjectTodoPatch, ProjectTodoRunKind, ProjectTodoRunRecord, ProjectTodoRunStatus,
    ProjectTodoStatus,
};
use tokio::task::JoinHandle;

use crate::{
    plan_gen::{close_run_if_running, forget_spawn},
    service::Deps,
};

/// stale run 收敛的统一留痕文案（sweep 与 execute 前置守卫共用）。
const STALE_RUN_NOTE: &str = "stale run converged: driver lost (restart/panic)";

/// spawn 后台驱动 + 小监控任务：await 驱动的 JoinHandle，若其以 panic 退出
/// 则做兜底收敛。正常完成路径的收敛由 drive 自身负责，监控只在 join 失败
/// 且 `is_panic()` 时出手（我们从不 abort 任务，Err 实际只可能是 panic，
/// 显式判一次更稳）。
pub(crate) fn spawn_run_driver<F, Fut>(
    deps: &Arc<Deps>,
    run_id: &str,
    todo_id: &str,
    kind: ProjectTodoRunKind,
    drive: F,
) where
    F: FnOnce() -> Fut,
    Fut: Future<Output = ()> + Send + 'static,
{
    let handle: JoinHandle<()> = tokio::spawn(drive());
    let deps = deps.clone();
    let run_id = run_id.to_string();
    let todo_id = todo_id.to_string();
    tokio::spawn(async move {
        if let Err(e) = handle.await {
            if e.is_panic() {
                tracing::error!(run_id = %run_id, todo_id = %todo_id, "project run driver panicked; converging run");
                converge_panicked_run(&deps, &run_id, &todo_id, kind).await;
            }
        }
    });
}

/// panic 兜底收敛：run → Failed，Execute 的 todo 也一并 Failed（todo 被
/// start_execute 置成 Running，不能悬死）；Plan 不占用 todo 状态故不动
/// todo。所有写库都走「只告警不冒泡」的条件 CAS。
pub(crate) async fn converge_panicked_run(
    deps: &Arc<Deps>,
    run_id: &str,
    todo_id: &str,
    kind: ProjectTodoRunKind,
) {
    // 驱动可能在 close_run(Done) 与 todo 回写的两个 await 之间 panic——run
    // 已终态时条件收敛不改写其标签/输出；但 todo 若仍 Running 必须补收敛，
    // 否则悬死。
    close_run_if_running(
        deps,
        run_id,
        ProjectTodoRunStatus::Failed,
        Some("run driver panicked".into()),
        None,
    )
    .await;
    if kind == ProjectTodoRunKind::Execute {
        fail_todo_if_running(deps, todo_id).await;
    }
    forget_spawn(deps, run_id);
}

/// 单条 stale run 的条件收敛（`sweep_stale_runs` 与 execute 前置守卫共用）：
/// CAS 把 Running run → Failed（stale 文案留痕 output）；Execute 赢家补收敛
/// todo（Plan 不占 todo 状态）。返回是否赢得 CAS——输家说明并发方已收敛，
/// 同样视为已处理。
pub(crate) async fn converge_stale_run(deps: &Arc<Deps>, run: &ProjectTodoRunRecord) -> bool {
    let won = close_run_if_running(
        deps,
        &run.id,
        ProjectTodoRunStatus::Failed,
        Some(STALE_RUN_NOTE.into()),
        None,
    )
    .await;
    if won && run.kind == ProjectTodoRunKind::Execute {
        fail_todo_if_running(deps, &run.todo_id).await;
    }
    won
}

/// lost-driver 取消收敛：cancel 找不到注册令牌而 run 行仍 Running 时调用。
/// 单进程假设下注册表即驱动存亡真源，且 run_id 在令牌注册之后才对外可见
/// （start_* 的返回晚于 spawn_run），故此处无需 grace 直接收敛，镜像驱动
/// 自身的取消语义：run → Cancelled；Execute 的 todo Running → Planned
/// （方案仍在，可再次执行）；Plan 不占 todo 状态。行缺失或已终态返回
/// false（无可取消）。若驱动实际还活着（双击 cancel 的良性竞态），其收尾
/// 的无条件 close_run 落在后、以真实终态覆盖，语义不受损。
pub(crate) async fn converge_lost_run(deps: &Arc<Deps>, run_id: &str) -> bool {
    let run = match deps.projects.get_todo_run(run_id).await {
        Ok(Some(run)) => run,
        Ok(None) => return false,
        Err(e) => {
            tracing::warn!(run_id, error = %e, "load run for lost-driver cancel failed");
            return false;
        }
    };
    if run.status != ProjectTodoRunStatus::Running {
        return false;
    }
    let won = close_run_if_running(
        deps,
        run_id,
        ProjectTodoRunStatus::Cancelled,
        Some("cancelled: driver lost (restart/panic)".into()),
        None,
    )
    .await;
    if won && run.kind == ProjectTodoRunKind::Execute {
        revert_todo_if_running(deps, &run.todo_id).await;
    }
    won
}

/// Running → Planned 条件回退：execute 取消语义（方案仍在，可再次执行）；
/// todo 已被别处收敛（非 Running）时不打花。写库失败只告警。
async fn revert_todo_if_running(deps: &Arc<Deps>, todo_id: &str) {
    let patch = ProjectTodoPatch {
        status: Some(ProjectTodoStatus::Planned),
        ..Default::default()
    };
    let now = opencoder_core::message::now_ms();
    if let Err(e) = deps
        .projects
        .patch_todo_when(todo_id, ProjectTodoStatus::Running, &patch, now)
        .await
    {
        tracing::warn!(todo_id, error = %e, "revert running todo during lost-driver cancel failed");
    }
}

/// Running → Failed 的条件 CAS：todo 已被驱动自身收敛（Done/Planned 回写
/// 已落地）时不打花终态；行缺失或已非 Running 静默跳过。
async fn fail_todo_if_running(deps: &Arc<Deps>, todo_id: &str) {
    let patch = ProjectTodoPatch {
        status: Some(ProjectTodoStatus::Failed),
        ..Default::default()
    };
    let now = opencoder_core::message::now_ms();
    if let Err(e) = deps
        .projects
        .patch_todo_when(todo_id, ProjectTodoStatus::Running, &patch, now)
        .await
    {
        tracing::warn!(todo_id, error = %e, "fail running todo during convergence failed");
    }
}

/// 机会式 stale run 清扫：running 行的 run_id 不在本进程注册表（驱动已
/// 不存在——重启丢失或兜底收敛后仍未终态）且 started_at 超过 grace 才判
/// 死。逐条条件收敛（仅 run 仍 Running 才改写）：run → Failed；CAS 赢了
/// 且 Execute 时把仍 Running 的 todo → Failed（Plan 不动 todo；todo 非
/// Running 说明已被别处收敛）。CAS 输掉说明并发方（驱动自身收尾或另一路
/// 兜底）已收敛该行——不计数、不碰 todo。返回收敛条数（读清单失败只告警
/// 并返回 0，调用方是读路径，不该被清扫抖动打断）。
pub(crate) async fn sweep_stale_runs(deps: &Arc<Deps>, grace_ms: i64) -> usize {
    let runs = match deps.projects.list_running_todo_runs().await {
        Ok(runs) => runs,
        Err(e) => {
            tracing::warn!(error = %e, "list running project runs failed; skip stale sweep");
            return 0;
        }
    };
    let now = opencoder_core::message::now_ms();
    let mut converged = 0usize;
    for run in runs {
        if deps.spawns.lock().unwrap().contains_key(&run.id) {
            continue; // 本进程驱动仍在跑
        }
        if now - run.started_at <= grace_ms {
            continue; // 宽限期内（慢启动 / 时钟抖动 / 收敛写入在途）
        }
        tracing::warn!(run_id = %run.id, "converging stale project run (driver lost)");
        if converge_stale_run(deps, &run).await {
            converged += 1;
        }
    }
    converged
}
