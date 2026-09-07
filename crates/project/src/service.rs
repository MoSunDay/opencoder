//! [`ProjectService`]：项目模块的运行时门面。持有全局 `Deps`（store +
//! project store + workdir + client override + spawn 注册表），对外提供
//! `start_plan` / `start_execute` / `cancel` / `overview` 四个入口。所有
//! 方法都是 `&self`：服务本身是共享的 `Send + Sync` 状态，运行态全部收敛
//! 到 store 与 spawns 注册表里，web 层可零成本在 AppState 中持有。

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
};

use anyhow::{bail, Context as _, Result};
use opencoder_llm::ChatStream;
use opencoder_store::{
    ProjectExecutorKind, ProjectStore, ProjectTodoRecord, ProjectTodoRunKind, ProjectTodoRunRecord,
    ProjectTodoStatus, Store, TASK_TYPE_PROJECT,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::context::ProjectContext;
use crate::executor::{resolve_brain, BrainHandoff, ResolvedExecutor};

/// 一次初始化后只读的共享依赖集。`spawns` 是运行中的 run_id → 取消令牌
/// 注册表（Mutex 包裹的普通 HashMap，跨 await 只做短临界区拷贝）。
/// `brain` 是能力库运行时（控制面注入；节点上没有——brain todo 在节点
/// 上会因缺运行时而拒启，需要控制面先预解析）。
pub struct Deps {
    pub store: Arc<dyn Store>,
    pub projects: Arc<dyn ProjectStore>,
    pub workdir: PathBuf,
    pub client_override: Option<Arc<dyn ChatStream>>,
    pub brain: Option<opencoder_brain::Runtime>,
    pub spawns: Mutex<HashMap<String, CancellationToken>>,
}

/// 执行启动时的执行器覆盖（控制面预解析结果）：跳过 todo 自带的三字段
/// 解析，直接按 kind + ref 驱动；capability/plan 是 brain 预解析的留痕。
/// 只允许 agent/team/dag——brain 不能被预解析成 brain（禁止嵌套）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutorOverride {
    /// 目标执行器（agent | team | dag；brain 在此被拒绝）。
    pub kind: ProjectExecutorKind,
    /// 执行器引用：team/dag 的资源名、agent 的代理名（可缺省）。
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub ref_: Option<String>,
    /// brain 预解析命中的能力 id（留痕用）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_id: Option<String>,
    /// brain 预解析使用的计划 id（留痕用）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<String>,
}

/// `TASK_TYPE_PROJECT` 常量在此模块被引用（SessionMeta.task_type），re-export
/// 方便上层（web 路由按 task_type 过滤会话列表）免开 store 命名空间。
pub const TASK_TYPE: &str = TASK_TYPE_PROJECT;

/// stale run 清扫宽限期：running 行不在本进程注册表且 `now - started_at`
/// 超过该时长才判死（重启丢驱动 / panic 兜底后仍未终态）。
const STALE_RUN_GRACE_MS: i64 = 300_000;

pub struct ProjectService {
    deps: OnceLock<Arc<Deps>>,
}

impl ProjectService {
    /// 便宜且同步：web 测试在构造 AppState 时不需要任何异步初始化。
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            deps: OnceLock::new(),
        })
    }

    /// 注入依赖（幂等拒绝二次初始化）。async 是为了给 web 层留出
    /// feature-gated 后端的构建空间；本函数本身不做 IO。
    pub async fn init(
        &self,
        store: Arc<dyn Store>,
        projects: Arc<dyn ProjectStore>,
        workdir: PathBuf,
        client_override: Option<Arc<dyn ChatStream>>,
        brain: Option<opencoder_brain::Runtime>,
    ) -> Result<()> {
        let deps = Arc::new(Deps {
            store,
            projects,
            workdir,
            client_override,
            brain,
            spawns: Mutex::new(HashMap::new()),
        });
        self.deps
            .set(deps)
            .map_err(|_| anyhow::anyhow!("project service already initialized"))
    }

    /// Current deps, or the "not initialized" error. Public so the web
    /// handlers can grab the typed store handles (`projects`) for the plain
    /// CRUD routes without going through the run-oriented service methods.
    pub fn require(&self) -> Result<Arc<Deps>> {
        self.deps
            .get()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("project service not initialized"))
    }

    /// 为 todo 生成（或重新生成）实施方案：spawn 一个 plan 直驱运行。
    pub async fn start_plan(&self, todo_id: &str) -> Result<String> {
        let deps = self.require()?;
        let todo = deps
            .projects
            .get_todo(todo_id)
            .await
            .context("load todo for plan")?
            .ok_or_else(|| anyhow::anyhow!("todo not found: {todo_id}"))?;
        if todo.status == ProjectTodoStatus::Running {
            bail!("todo is running");
        }
        let cx = build_context(&deps, &todo).await?;
        let run_id = format!("prun-{}", ulid::Ulid::new());
        let version = deps.projects.next_todo_version(todo_id).await?;
        let now = opencoder_core::message::now_ms();
        deps.projects
            .create_todo_run(&ProjectTodoRunRecord {
                id: run_id.clone(),
                todo_id: todo_id.to_string(),
                kind: ProjectTodoRunKind::Plan,
                version,
                plan_md: None,
                output_md: None,
                agent: "plan".into(),
                // plan 运行恒为 plan 代理直驱，执行器维度固定 Agent（与
                // todo 的执行器无关）。
                executor_kind: ProjectExecutorKind::Agent,
                capability_id: None,
                plan_id: None,
                output_ref: None,
                session_id: None,
                status: opencoder_store::ProjectTodoRunStatus::Running,
                started_at: now,
                finished_at: None,
                created_at: now,
            })
            .await
            .context("create plan run")?;
        let token = spawn_run(&deps, &run_id);
        let drive_todo = todo.clone();
        let drive_cx = cx;
        let drive_deps = deps.clone();
        let drive_run = run_id.clone();
        // spawn 驱动 + panic 监控：驱动 panic 时不留 running 悬行。
        crate::recover::spawn_run_driver(
            &deps,
            &run_id,
            todo_id,
            ProjectTodoRunKind::Plan,
            move || crate::plan_gen::drive(drive_deps, drive_run, drive_todo, drive_cx, token),
        );
        Ok(run_id)
    }

    /// 按 todo 的现行方案驱动一次执行运行（无覆盖）。
    pub async fn start_execute(&self, todo_id: &str) -> Result<String> {
        self.start_execute_with(todo_id, None).await
    }

    /// 同上，但允许控制面携带执行器覆盖（brain 预解析结果）。spawn 之前
    /// 先把 todo 置为 Running（崩溃时由 store 状态自证）。执行器解析在
    /// claim 之前完成：解析失败（brain 缺运行时、team/dag 缺目标等）
    /// 直接向上抛，不留半启动状态；brain 解析可能触发 LLM 路由调用，
    /// 这一窗口不含任何已 claim 的状态，代价可接受。
    pub async fn start_execute_with(
        &self,
        todo_id: &str,
        override_: Option<ExecutorOverride>,
    ) -> Result<String> {
        let deps = self.require()?;
        let todo = deps
            .projects
            .get_todo(todo_id)
            .await
            .context("load todo for execute")?
            .ok_or_else(|| anyhow::anyhow!("todo not found: {todo_id}"))?;
        if todo.status == ProjectTodoStatus::Running {
            bail!("todo is running");
        }
        if todo.plan_md.is_none() {
            bail!("todo has no plan — generate one first");
        }
        ensure_no_plan_in_flight(&deps, todo_id).await?;
        let cx = build_context(&deps, &todo).await?;
        // claim 前解析执行器：brain 走运行时解析（含钉住/覆盖捷径），
        // 其余按 todo 三字段纯解析。留痕（capability/plan）随解析产出。
        let (resolved, trace) = if todo.executor_kind == ProjectExecutorKind::Brain {
            resolve_brain(&deps, &todo, &cx, override_.as_ref()).await?
        } else {
            (
                crate::executor::resolve(&todo, override_.as_ref())?,
                Default::default(),
            )
        };
        let run_id = format!("prun-{}", ulid::Ulid::new());
        let version = deps.projects.next_todo_version(todo_id).await?;
        let now = opencoder_core::message::now_ms();
        let run = ProjectTodoRunRecord {
            id: run_id.clone(),
            todo_id: todo_id.to_string(),
            kind: ProjectTodoRunKind::Execute,
            version,
            // 执行起点的方案快照：后续 plan 重新生成不会改写本次执行的
            // 留痕（前置检查已保证 Some）。
            plan_md: todo.plan_md.clone(),
            output_md: None,
            agent: run_agent_label(&resolved, &todo),
            executor_kind: resolved.kind,
            capability_id: trace.capability_id.clone(),
            plan_id: trace.plan_id.clone(),
            output_ref: None,
            session_id: None,
            status: opencoder_store::ProjectTodoRunStatus::Running,
            started_at: now,
            finished_at: None,
            created_at: now,
        };
        // Store 的单事务 claim + INSERT 关死两个提交之间的崩溃窗口；并发
        // 双击/多实例只有一个调用方能赢，插入失败会连同 claim 一起回滚。
        if !deps
            .projects
            .claim_todo_running_with_run(&run, now)
            .await
            .context("claim todo and create execute run")?
        {
            bail!("todo is running");
        }
        let token = spawn_run(&deps, &run_id);
        let drive_deps = deps.clone();
        let drive_run = run_id.clone();
        // brain todo 以 Brain 标记 + 派发交接进派发：claim 前解析出的
        // override 随行——brain_drive 重解析直接采纳该纯函数分支（节点
        // 无 brain 运行时也可执行），留痕沿用 claim 前解析（单一事实源）；
        // 其余 kind 直接携带解析结果。
        let (drive_resolved, brain_handoff) = if todo.executor_kind == ProjectExecutorKind::Brain {
            (
                ResolvedExecutor {
                    kind: ProjectExecutorKind::Brain,
                    ref_: None,
                },
                Some(BrainHandoff {
                    override_: override_.clone(),
                    trace: Some(trace),
                }),
            )
        } else {
            (resolved, None)
        };
        // spawn 驱动 + panic 监控：驱动 panic 时 run/todo 一并收敛。
        crate::recover::spawn_run_driver(
            &deps,
            &run_id,
            todo_id,
            ProjectTodoRunKind::Execute,
            move || {
                crate::executor::drive(
                    drive_deps,
                    drive_run,
                    todo,
                    cx,
                    version,
                    drive_resolved,
                    brain_handoff,
                    token,
                )
            },
        );
        Ok(run_id)
    }

    /// 取消一个运行中的 run。返回是否实际取消：注册令牌存在并已触发
    /// cancel；或（lost-driver 形态）令牌不在注册表而 run 行仍 Running——
    /// 驱动已丢失（重启/panic 收敛后仍未终态），此时机会式收敛 run →
    /// Cancelled（execute 的 todo 回退 Planned），不等 stale grace。
    /// 行缺失或已终态返回 false。
    pub async fn cancel(&self, run_id: &str) -> Result<bool> {
        // 未初始化时没有可取消的运行：按「未找到」处理，而不是报错，
        // 这样 cancel 永远是安全幂等的。
        let Some(deps) = self.deps.get() else {
            return Ok(false);
        };
        let token = deps.spawns.lock().unwrap().remove(run_id);
        if let Some(token) = token {
            token.cancel();
            return Ok(true);
        }
        Ok(crate::recover::converge_lost_run(deps, run_id).await)
    }

    /// 全量树形总览：目标(含里程碑(含待办)) + 无里程碑的 backlog。
    pub async fn overview(&self) -> Result<Value> {
        let deps = self.require()?;
        // 机会式 stale run 清扫（无后台定时器）：读路径触发，失败只告警，
        // 不让总览因为清扫抖动而 500（镜像 converge_lost_node_tasks 思路）。
        let _ = crate::recover::sweep_stale_runs(&deps, STALE_RUN_GRACE_MS).await;
        let goals = deps.projects.list_goals().await.context("list goals")?;
        let milestones = deps
            .projects
            .list_milestones(None)
            .await
            .context("list milestones")?;
        let todos = deps.projects.list_todos(None).await.context("list todos")?;
        let mut backlog = Vec::new();
        for todo in &todos {
            if todo.milestone_id.is_none() {
                backlog.push(serde_json::to_value(todo)?);
            }
        }
        let mut goals_json = Vec::with_capacity(goals.len());
        for goal in &goals {
            let mut node = serde_json::to_value(goal)?;
            let mut ms_json = Vec::new();
            for ms in milestones.iter().filter(|m| m.goal_id == goal.id) {
                let mut ms_node = serde_json::to_value(ms)?;
                let mut todo_json = Vec::new();
                for todo in todos
                    .iter()
                    .filter(|t| t.milestone_id.as_deref() == Some(ms.id.as_str()))
                {
                    todo_json.push(serde_json::to_value(todo)?);
                }
                ms_node["todos"] = Value::Array(todo_json);
                ms_json.push(ms_node);
            }
            node["milestones"] = Value::Array(ms_json);
            goals_json.push(node);
        }
        Ok(json!({ "goals": goals_json, "backlog": backlog }))
    }
}

/// 注册取消令牌并返回其克隆（drive 结束时自行摘除）。
fn spawn_run(deps: &Arc<Deps>, run_id: &str) -> CancellationToken {
    let token = CancellationToken::new();
    deps.spawns
        .lock()
        .unwrap()
        .insert(run_id.to_string(), token.clone());
    token
}

/// plan/execute 互斥（正向）：plan 重新生成进行中不允许启动执行——plan
/// 收尾回写与 execute 的 Running 状态会互踩。此检查关掉主窗口；plan 收
/// 尾的条件回写（plan_gen::commit_plan_output）兜住「检查→claim」之间
/// 的残余竞态。反向（执行中不可重 plan）由 todo.status 检查保证。
/// 「进行中」以本进程注册表为准：崩溃/重启残留的 stale plan 行（不在
/// 注册表且超 grace）不阻塞执行——机会式收敛后放行，消灭「崩溃后必须
/// 等总览触发 sweep」的死角；grace 内的未注册行仍保守拒绝（并发
/// start_plan 在 create→注册之间的毫秒级窗口靠 grace 兜住）。
async fn ensure_no_plan_in_flight(deps: &Arc<Deps>, todo_id: &str) -> Result<()> {
    let now = opencoder_core::message::now_ms();
    let mut plan_in_flight = false;
    for run in deps
        .projects
        .list_running_todo_runs()
        .await
        .context("list running runs for execute")?
    {
        if run.todo_id != todo_id || run.kind != ProjectTodoRunKind::Plan {
            continue;
        }
        if deps.spawns.lock().unwrap().contains_key(&run.id)
            || now - run.started_at <= STALE_RUN_GRACE_MS
        {
            plan_in_flight = true;
        } else {
            tracing::warn!(run_id = %run.id, "converging stale plan run before execute");
            crate::recover::converge_stale_run(deps, &run).await;
        }
    }
    if plan_in_flight {
        bail!("todo plan generation is in progress");
    }
    Ok(())
}

/// 执行器展示名：ref 优先，其次内联 spec 的 `name` 字段（team spec 有，
/// dag spec 有；brain routes 没有），最后落到 todo id。仅作 run 行标签。
fn executor_display_name(resolved: &ResolvedExecutor, todo: &ProjectTodoRecord) -> String {
    if let Some(name) = resolved
        .ref_
        .as_deref()
        .or(todo.executor_ref.as_deref())
        .map(str::trim)
        .filter(|n| !n.is_empty())
    {
        return name.to_string();
    }
    todo.executor_spec
        .as_deref()
        .and_then(|spec| serde_json::from_str::<Value>(spec).ok())
        .and_then(|v| v.get("name").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_else(|| todo.id.clone())
}

/// run 行的 agent 标签：agent 携带解析出的代理名优先（brain 路由/override
/// 会带名），否则沿用 todo.agent；team/dag 带上执行器名（`team:<名>` /
/// `dag:<名>`）；brain 标记不会出现在已解析结果里（resolve/resolve_brain
/// 都不产出 Brain），此分支只是完备性兜底。
fn run_agent_label(resolved: &ResolvedExecutor, todo: &ProjectTodoRecord) -> String {
    match resolved.kind {
        ProjectExecutorKind::Agent => resolved.ref_.clone().unwrap_or_else(|| todo.agent.clone()),
        ProjectExecutorKind::Team => format!("team:{}", executor_display_name(resolved, todo)),
        ProjectExecutorKind::Dag => format!("dag:{}", executor_display_name(resolved, todo)),
        ProjectExecutorKind::Brain => "brain".into(),
    }
}

/// 组装 plan/execute 提示词所需的目标→里程碑→待办上下文。里程碑与目标
/// 均为 best-effort：行缺失时省略对应段落，目标缺失时用占位标题。
async fn build_context(deps: &Arc<Deps>, todo: &ProjectTodoRecord) -> Result<ProjectContext> {
    let milestone = match &todo.milestone_id {
        Some(mid) => deps
            .projects
            .list_milestones(None)
            .await
            .context("list milestones")?
            .into_iter()
            .find(|m| m.id == *mid),
        None => None,
    };
    let goal = match &milestone {
        Some(ms) => deps
            .projects
            .list_goals()
            .await
            .context("list goals")?
            .into_iter()
            .find(|g| g.id == ms.goal_id),
        None => None,
    };
    Ok(ProjectContext {
        goal_title: goal
            .as_ref()
            .map(|g| g.title.clone())
            .unwrap_or_else(|| "未命名目标".into()),
        goal_detail_md: goal.as_ref().and_then(|g| g.detail_md.clone()),
        milestone_title: milestone.as_ref().map(|m| m.title.clone()),
        milestone_detail_md: milestone.as_ref().and_then(|m| m.detail_md.clone()),
        todo_title: todo.title.clone(),
        todo_draft: todo.draft.clone(),
    })
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
