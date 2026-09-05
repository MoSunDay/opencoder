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
    ProjectStore, ProjectTodoPatch, ProjectTodoRecord, ProjectTodoRunKind, ProjectTodoRunRecord,
    ProjectTodoStatus, Store, TASK_TYPE_PROJECT,
};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::context::ProjectContext;

/// 一次初始化后只读的共享依赖集。`spawns` 是运行中的 run_id → 取消令牌
/// 注册表（Mutex 包裹的普通 HashMap，跨 await 只做短临界区拷贝）。
pub struct Deps {
    pub store: Arc<dyn Store>,
    pub projects: Arc<dyn ProjectStore>,
    pub workdir: PathBuf,
    pub client_override: Option<Arc<dyn ChatStream>>,
    pub spawns: Mutex<HashMap<String, CancellationToken>>,
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
    ) -> Result<()> {
        let deps = Arc::new(Deps {
            store,
            projects,
            workdir,
            client_override,
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

    /// 按 todo 的现行方案驱动一次执行运行：新会话或 resume 既有会话。
    /// spawn 之前先把 todo 置为 Running（崩溃时由 store 状态自证）。
    pub async fn start_execute(&self, todo_id: &str) -> Result<String> {
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
        // plan/execute 互斥（正向）：plan 重新生成进行中不允许启动执行——plan 收尾
        // 回写与 execute 的 Running 状态会互踩。此检查关掉主窗口；plan 收尾的
        // 条件回写（plan_gen::commit_plan_output）兜住「检查→claim」之间的残余
        // 竞态。反向（执行中不可重 plan）由上面的 status 检查保证。
        // 「进行中」以本进程注册表为准：崩溃/重启残留的 stale plan 行（不在
        // 注册表且超 grace）不阻塞执行——机会式收敛后放行，消灭「崩溃后必须
        // 等总览触发 sweep」的死角；grace 内的未注册行仍保守拒绝（并发
        // start_plan 在 create→注册之间的毫秒级窗口靠 grace 兜住）。
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
                crate::recover::converge_stale_run(&deps, &run).await;
            }
        }
        if plan_in_flight {
            bail!("todo plan generation is in progress");
        }
        let cx = build_context(&deps, &todo).await?;
        let run_id = format!("prun-{}", ulid::Ulid::new());
        let version = deps.projects.next_todo_version(todo_id).await?;
        let now = opencoder_core::message::now_ms();
        // Expected-status CAS：单条条件 UPDATE 关死「读后写」的 TOCTOU 窗口
        // ——并发双击/多实例下只有一个调用方能赢；输家（未找到或已在
        // running）与前置检查里的 "todo is running" 同语义报错。
        if !deps
            .projects
            .claim_todo_running(todo_id, now)
            .await
            .context("claim todo running")?
        {
            bail!("todo is running");
        }
        if let Err(e) = deps
            .projects
            .create_todo_run(&ProjectTodoRunRecord {
                id: run_id.clone(),
                todo_id: todo_id.to_string(),
                kind: ProjectTodoRunKind::Execute,
                version,
                // 执行起点的方案快照：后续 plan 重新生成不会改写本次执行的
                // 留痕（前置检查已保证 Some）。
                plan_md: todo.plan_md.clone(),
                output_md: None,
                agent: todo.agent.clone(),
                session_id: None,
                status: opencoder_store::ProjectTodoRunStatus::Running,
                started_at: now,
                finished_at: None,
                created_at: now,
            })
            .await
        {
            // claim 补偿回滚：create 失败时 run 行不存在，sweep 只扫 run 行，这条
            // 悬死 Running 永远无法自愈。条件 CAS（仍 Running 才改写）放回 claim
            // 前的状态；回滚自身失败只告警，原始错误照常上抛。
            let rollback = ProjectTodoPatch {
                status: Some(todo.status),
                ..Default::default()
            };
            if let Err(rb) = deps
                .projects
                .patch_todo_when(todo_id, ProjectTodoStatus::Running, &rollback, now)
                .await
            {
                tracing::error!(todo_id, error = %rb, "rollback todo claim after create run failure failed");
            }
            return Err(e).context("create execute run");
        }
        let token = spawn_run(&deps, &run_id);
        let drive_deps = deps.clone();
        let drive_run = run_id.clone();
        // spawn 驱动 + panic 监控：驱动 panic 时 run/todo 一并收敛。
        crate::recover::spawn_run_driver(
            &deps,
            &run_id,
            todo_id,
            ProjectTodoRunKind::Execute,
            move || crate::execute::drive(drive_deps, drive_run, todo, cx, version, token),
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
mod tests {
    use super::*;
    use opencoder_store::{LibsqlStore, ProjectTodoRunPatch, ProjectTodoRunStatus as RunStatus};

    use crate::recover;

    #[tokio::test]
    async fn uninitialized_require_and_read_paths_error_cleanly() {
        let service = ProjectService::new();
        assert!(service.start_plan("t1").await.is_err());
        assert!(service.overview().await.is_err());
    }

    #[tokio::test]
    async fn cancel_unknown_or_uninitialized_returns_false() {
        let service = ProjectService::new();
        assert!(!service.cancel("nope").await.unwrap());
    }

    // ---- recover: panic 兜底 + stale 清扫（手工构造 Deps，内存库） ----

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
        p.create_todo(&ProjectTodoRecord {
            id: id.into(),
            milestone_id: None,
            title: format!("待办 {id}"),
            draft: "草稿".into(),
            plan_md: Some("# 方案".into()),
            status,
            agent: "act".into(),
            active_session_id: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    }

    async fn seed_run(p: &Arc<LibsqlStore>, id: &str, todo_id: &str, kind: ProjectTodoRunKind) {
        seed_run_at(p, id, todo_id, kind, 1000).await;
    }

    async fn seed_run_at(
        p: &Arc<LibsqlStore>,
        id: &str,
        todo_id: &str,
        kind: ProjectTodoRunKind,
        started_at: i64,
    ) {
        let version = p.next_todo_version(todo_id).await.unwrap();
        p.create_todo_run(&ProjectTodoRunRecord {
            id: id.into(),
            todo_id: todo_id.into(),
            kind,
            version,
            plan_md: None,
            output_md: None,
            agent: "act".into(),
            session_id: None,
            status: RunStatus::Running,
            started_at,
            finished_at: None,
            created_at: started_at,
        })
        .await
        .unwrap();
    }

    async fn todo_status(p: &Arc<LibsqlStore>, id: &str) -> ProjectTodoStatus {
        p.get_todo(id).await.unwrap().unwrap().status
    }

    async fn run_status(p: &Arc<LibsqlStore>, id: &str) -> RunStatus {
        p.get_todo_run(id).await.unwrap().unwrap().status
    }

    #[tokio::test]
    async fn panic_convergence_fails_run_execute_todo_and_forgets_spawn() {
        let (_dir, deps, p) = test_deps().await;
        seed_todo(&p, "t1", ProjectTodoStatus::Running).await;
        seed_run(&p, "r1", "t1", ProjectTodoRunKind::Execute).await;
        deps.spawns
            .lock()
            .unwrap()
            .insert("r1".into(), CancellationToken::new());

        recover::converge_panicked_run(&deps, "r1", "t1", ProjectTodoRunKind::Execute).await;

        let run = p.get_todo_run("r1").await.unwrap().unwrap();
        assert_eq!(run.status, RunStatus::Failed);
        assert_eq!(run.output_md.as_deref(), Some("run driver panicked"));
        assert!(run.finished_at.is_some(), "close_run stamps finished_at");
        assert_eq!(todo_status(&p, "t1").await, ProjectTodoStatus::Failed);
        assert!(
            !deps.spawns.lock().unwrap().contains_key("r1"),
            "panic convergence removes the spawn token"
        );
    }

    #[tokio::test]
    async fn panic_convergence_leaves_plan_todo_untouched() {
        let (_dir, deps, p) = test_deps().await;
        seed_todo(&p, "t1", ProjectTodoStatus::Planned).await;
        seed_run(&p, "r1", "t1", ProjectTodoRunKind::Plan).await;

        recover::converge_panicked_run(&deps, "r1", "t1", ProjectTodoRunKind::Plan).await;

        assert_eq!(run_status(&p, "r1").await, RunStatus::Failed);
        assert_eq!(
            todo_status(&p, "t1").await,
            ProjectTodoStatus::Planned,
            "plan runs do not own the todo status"
        );
    }

    #[tokio::test]
    async fn sweep_converges_only_unregistered_runs_past_grace() {
        let (_dir, deps, p) = test_deps().await;
        // 三个 execute/plan 各一：过期未注册（要收敛）、刚启动未注册（宽限）、
        // 过期但已注册（本进程驱动仍在跑）。另有一个过期 run 挂在非 Running
        // 的 todo 上：run 收敛但 todo 不被动。
        seed_todo(&p, "t-exec", ProjectTodoStatus::Running).await;
        seed_todo(&p, "t-plan", ProjectTodoStatus::Planned).await;
        seed_todo(&p, "t-live", ProjectTodoStatus::Running).await;
        seed_todo(&p, "t-done", ProjectTodoStatus::Done).await;
        seed_run(&p, "r-exec", "t-exec", ProjectTodoRunKind::Execute).await;
        seed_run(&p, "r-plan", "t-plan", ProjectTodoRunKind::Plan).await;
        seed_run(&p, "r-live", "t-live", ProjectTodoRunKind::Execute).await;
        seed_run(&p, "r-done", "t-done", ProjectTodoRunKind::Execute).await;
        // 「刚刚启动」的未注册 running run：宽限期内不动（正常启动到
        // spawn 注册之间存在毫秒级窗口，靠 grace 兜住）。
        let fresh_started = opencoder_core::message::now_ms();
        seed_run_at(
            &p,
            "r-fresh",
            "t-exec",
            ProjectTodoRunKind::Execute,
            fresh_started,
        )
        .await;
        deps.spawns
            .lock()
            .unwrap()
            .insert("r-live".into(), CancellationToken::new());

        let converged = recover::sweep_stale_runs(&deps, 300_000).await;
        assert_eq!(converged, 3, "r-exec + r-plan + r-done converge");

        let exec = p.get_todo_run("r-exec").await.unwrap().unwrap();
        assert_eq!(exec.status, RunStatus::Failed);
        assert_eq!(
            exec.output_md.as_deref(),
            Some("stale run converged: driver lost (restart/panic)")
        );
        assert_eq!(todo_status(&p, "t-exec").await, ProjectTodoStatus::Failed);
        assert_eq!(run_status(&p, "r-plan").await, RunStatus::Failed);
        assert_eq!(
            todo_status(&p, "t-plan").await,
            ProjectTodoStatus::Planned,
            "plan run convergence never touches the todo"
        );
        assert_eq!(
            todo_status(&p, "t-done").await,
            ProjectTodoStatus::Done,
            "non-running todo stays as-is even for execute runs"
        );
        assert_eq!(
            run_status(&p, "r-fresh").await,
            RunStatus::Running,
            "inside the grace window: kept"
        );
        assert_eq!(
            run_status(&p, "r-live").await,
            RunStatus::Running,
            "registered token: this process still owns the driver"
        );
        assert_eq!(todo_status(&p, "t-live").await, ProjectTodoStatus::Running);
    }

    #[tokio::test]
    async fn start_execute_rejects_while_plan_run_in_flight() {
        let store = Arc::new(LibsqlStore::open_memory().await.unwrap());
        let dir = tempfile::tempdir().unwrap();
        let service = ProjectService::new();
        service
            .init(store.clone(), store.clone(), dir.path().to_path_buf(), None)
            .await
            .unwrap();
        seed_todo(&store, "t1", ProjectTodoStatus::Planned).await;
        seed_run(&store, "r-plan", "t1", ProjectTodoRunKind::Plan).await;
        // 真活驱动：令牌在注册表里（seed 的 started_at 很老，只有注册表
        // 命中才能证明「进行中」）。
        service
            .deps
            .get()
            .unwrap()
            .spawns
            .lock()
            .unwrap()
            .insert("r-plan".into(), CancellationToken::new());

        let err = service.start_execute("t1").await.unwrap_err();
        assert!(err.to_string().contains("plan"), "got: {err:#}");
        assert_eq!(
            todo_status(&store, "t1").await,
            ProjectTodoStatus::Planned,
            "plan run 进行中不拿走 execute claim"
        );
    }

    #[tokio::test]
    async fn panic_convergence_keeps_terminal_run_label() {
        let (_dir, deps, p) = test_deps().await;
        seed_todo(&p, "t1", ProjectTodoStatus::Done).await;
        seed_run(&p, "r1", "t1", ProjectTodoRunKind::Execute).await;
        // 驱动已在 panic 前把 run 收敛到 Done 并回写 todo（Done）。
        p.patch_todo_run(
            "r1",
            &ProjectTodoRunPatch {
                status: Some(RunStatus::Done),
                output_md: Some("执行完成".into()),
                finished_at: Some(2000),
                ..Default::default()
            },
            2000,
        )
        .await
        .unwrap();
        deps.spawns
            .lock()
            .unwrap()
            .insert("r1".into(), CancellationToken::new());

        recover::converge_panicked_run(&deps, "r1", "t1", ProjectTodoRunKind::Execute).await;

        let run = p.get_todo_run("r1").await.unwrap().unwrap();
        assert_eq!(run.status, RunStatus::Done, "终态标签不被兜底改写");
        assert_eq!(
            run.output_md.as_deref(),
            Some("执行完成"),
            "原始输出不被 \"run driver panicked\" 打花"
        );
        assert_eq!(todo_status(&p, "t1").await, ProjectTodoStatus::Done);
        assert!(
            !deps.spawns.lock().unwrap().contains_key("r1"),
            "panic convergence removes the spawn token"
        );
    }

    #[tokio::test]
    async fn panic_convergence_after_run_done_still_fails_stuck_running_todo() {
        let (_dir, deps, p) = test_deps().await;
        // 「close_run(Done) 与 todo 回写之间 panic」形状：run 已 Done、todo
        // 悬在 Running——run 标签不动，todo 必须补收敛为 Failed。
        seed_todo(&p, "t1", ProjectTodoStatus::Running).await;
        seed_run(&p, "r1", "t1", ProjectTodoRunKind::Execute).await;
        p.patch_todo_run(
            "r1",
            &ProjectTodoRunPatch {
                status: Some(RunStatus::Done),
                output_md: Some("执行完成".into()),
                finished_at: Some(2000),
                ..Default::default()
            },
            2000,
        )
        .await
        .unwrap();

        recover::converge_panicked_run(&deps, "r1", "t1", ProjectTodoRunKind::Execute).await;

        assert_eq!(run_status(&p, "r1").await, RunStatus::Done);
        assert_eq!(
            todo_status(&p, "t1").await,
            ProjectTodoStatus::Failed,
            "run 已终态但 todo 仍 Running：必须补收敛，不悬死"
        );
    }

    #[tokio::test]
    async fn start_execute_blocks_unregistered_plan_run_within_grace() {
        let store = Arc::new(LibsqlStore::open_memory().await.unwrap());
        let dir = tempfile::tempdir().unwrap();
        let service = ProjectService::new();
        service
            .init(store.clone(), store.clone(), dir.path().to_path_buf(), None)
            .await
            .unwrap();
        seed_todo(&store, "t1", ProjectTodoStatus::Planned).await;
        // 未注册但刚起步：可能是并发 start_plan 的 create→注册窗口，保守拒绝。
        seed_run_at(
            &store,
            "r-fresh",
            "t1",
            ProjectTodoRunKind::Plan,
            opencoder_core::message::now_ms(),
        )
        .await;

        let err = service.start_execute("t1").await.unwrap_err();
        assert!(err.to_string().contains("plan"), "got: {err:#}");
    }

    #[tokio::test]
    async fn start_execute_converges_stale_plan_run_past_grace() {
        let store = Arc::new(LibsqlStore::open_memory().await.unwrap());
        let dir = tempfile::tempdir().unwrap();
        let service = ProjectService::new();
        service
            .init(store.clone(), store.clone(), dir.path().to_path_buf(), None)
            .await
            .unwrap();
        seed_todo(&store, "t1", ProjectTodoStatus::Planned).await;
        // 崩溃残留：不在注册表且超 grace 的 running plan 行——不阻塞执行，
        // 机会式收敛为 Failed（sweep 同款文案）。
        seed_run_at(
            &store,
            "r-stale",
            "t1",
            ProjectTodoRunKind::Plan,
            opencoder_core::message::now_ms() - STALE_RUN_GRACE_MS - 1,
        )
        .await;

        let run_id = service.start_execute("t1").await.unwrap();
        assert_ne!(run_id, "r-stale", "新 execute run，而非残留 plan 行");
        let stale = store.get_todo_run("r-stale").await.unwrap().unwrap();
        assert_eq!(stale.status, RunStatus::Failed);
        assert_eq!(
            stale.output_md.as_deref(),
            Some("stale run converged: driver lost (restart/panic)")
        );
    }

    #[tokio::test]
    async fn cancel_converges_lost_driver_execute_run_to_cancelled() {
        let (_dir, deps, p) = test_deps().await;
        seed_todo(&p, "t1", ProjectTodoStatus::Running).await;
        seed_run(&p, "r1", "t1", ProjectTodoRunKind::Execute).await;
        // 服务持独立（空）注册表：r1 不在其中 = 驱动已丢失。
        let service = ProjectService::new();
        service
            .init(
                deps.store.clone(),
                deps.projects.clone(),
                deps.workdir.clone(),
                None,
            )
            .await
            .unwrap();

        assert!(service.cancel("r1").await.unwrap());

        let run = p.get_todo_run("r1").await.unwrap().unwrap();
        assert_eq!(run.status, RunStatus::Cancelled);
        assert!(run.finished_at.is_some(), "converge 落 finished_at");
        assert_eq!(
            todo_status(&p, "t1").await,
            ProjectTodoStatus::Planned,
            "lost-driver 取消回退 Planned（方案仍在，可再次执行）"
        );
    }

    #[tokio::test]
    async fn cancel_terminal_or_missing_run_stays_false() {
        let (_dir, deps, p) = test_deps().await;
        seed_todo(&p, "t1", ProjectTodoStatus::Done).await;
        seed_run(&p, "r1", "t1", ProjectTodoRunKind::Execute).await;
        p.patch_todo_run(
            "r1",
            &ProjectTodoRunPatch {
                status: Some(RunStatus::Done),
                finished_at: Some(2000),
                ..Default::default()
            },
            2000,
        )
        .await
        .unwrap();
        let service = ProjectService::new();
        service
            .init(
                deps.store.clone(),
                deps.projects.clone(),
                deps.workdir.clone(),
                None,
            )
            .await
            .unwrap();

        assert!(!service.cancel("r1").await.unwrap(), "已终态：无可取消");
        assert!(
            !service.cancel("missing").await.unwrap(),
            "行缺失：无可取消"
        );
        assert_eq!(run_status(&p, "r1").await, RunStatus::Done, "终态不被打花");
    }
}
