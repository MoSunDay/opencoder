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
    ProjectStore, ProjectTodoPatch, ProjectTodoRecord, ProjectTodoRunKind,
    ProjectTodoRunRecord, ProjectTodoStatus, Store, TASK_TYPE_PROJECT,
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
        tokio::spawn(async move {
            crate::plan_gen::drive(drive_deps, drive_run, drive_todo, drive_cx, token).await;
        });
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
        let cx = build_context(&deps, &todo).await?;
        let run_id = format!("prun-{}", ulid::Ulid::new());
        let version = deps.projects.next_todo_version(todo_id).await?;
        let now = opencoder_core::message::now_ms();
        deps.projects
            .patch_todo(
                todo_id,
                &ProjectTodoPatch {
                    title: None,
                    draft: None,
                    plan_md: None,
                    status: Some(ProjectTodoStatus::Running),
                    agent: None,
                    milestone_id: None,
                    active_session_id: None,
                },
                now,
            )
            .await
            .context("mark todo running")?;
        deps.projects
            .create_todo_run(&ProjectTodoRunRecord {
                id: run_id.clone(),
                todo_id: todo_id.to_string(),
                kind: ProjectTodoRunKind::Execute,
                version,
                plan_md: None,
                output_md: None,
                agent: todo.agent.clone(),
                session_id: None,
                status: opencoder_store::ProjectTodoRunStatus::Running,
                started_at: now,
                finished_at: None,
                created_at: now,
            })
            .await
            .context("create execute run")?;
        let token = spawn_run(&deps, &run_id);
        let drive_deps = deps.clone();
        let drive_run = run_id.clone();
        tokio::spawn(async move {
            crate::execute::drive(drive_deps, drive_run, todo, cx, version, token).await;
        });
        Ok(run_id)
    }

    /// 取消一个运行中的 run。返回是否找到了该 run 的注册令牌。
    pub async fn cancel(&self, run_id: &str) -> Result<bool> {
        // 未初始化时没有可取消的运行：按「未找到」处理，而不是报错，
        // 这样 cancel 永远是安全幂等的。
        let Some(deps) = self.deps.get() else {
            return Ok(false);
        };
        let token = deps.spawns.lock().unwrap().remove(run_id);
        if let Some(token) = token {
            token.cancel();
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// 全量树形总览：目标(含里程碑(含待办)) + 无里程碑的 backlog。
    pub async fn overview(&self) -> Result<Value> {
        let deps = self.require()?;
        let goals = deps.projects.list_goals().await.context("list goals")?;
        let milestones = deps
            .projects
            .list_milestones(None)
            .await
            .context("list milestones")?;
        let todos = deps
            .projects
            .list_todos(None)
            .await
            .context("list todos")?;
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
                for todo in todos.iter().filter(|t| t.milestone_id.as_deref() == Some(ms.id.as_str())) {
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
}
