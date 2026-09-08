//! Durable attempt admission is separate from starting a driver.
use crate::service::ExecutorOverride;
use crate::{
    executor::{resolve_brain, BrainHandoff, BrainTrace, ResolvedExecutor},
    service::{build_context, ensure_no_plan_in_flight, run_agent_label, spawn_run},
    ProjectService,
};
use anyhow::{bail, Context, Result};
use opencoder_store::{
    ProjectExecutorKind, ProjectTodoRunKind, ProjectTodoRunRecord, ProjectTodoRunStatus,
    ProjectTodoStatus,
};
use serde_json::{json, Value};

impl ProjectService {
    pub async fn start_plan(&self, todo_id: &str) -> Result<String> {
        self.start_attempt(todo_id, ProjectTodoRunKind::Plan, None, None, json!({}))
            .await
    }
    pub async fn start_execute(&self, todo_id: &str) -> Result<String> {
        self.start_execute_with(todo_id, None).await
    }
    pub async fn start_execute_with(
        &self,
        todo_id: &str,
        override_: Option<ExecutorOverride>,
    ) -> Result<String> {
        self.start_attempt(
            todo_id,
            ProjectTodoRunKind::Execute,
            None,
            override_,
            json!({}),
        )
        .await
    }
    pub async fn start_attempt(
        &self,
        todo_id: &str,
        kind: ProjectTodoRunKind,
        id: Option<&str>,
        override_: Option<ExecutorOverride>,
        request: Value,
    ) -> Result<String> {
        let id = id
            .map(str::to_owned)
            .unwrap_or_else(|| format!("prun-{}", ulid::Ulid::new()));
        let existing = self.require()?.projects.get_todo_run(&id).await?.is_some();
        let run = self
            .reserve_attempt(todo_id, kind, &id, override_, request)
            .await?;
        if !existing {
            self.drive_reserved(&run.id).await?;
        }
        Ok(run.id)
    }
    /// A retry is resolved before looking at mutable definitions or resources.
    pub async fn accepted_attempt(
        &self,
        todo_id: &str,
        kind: ProjectTodoRunKind,
        id: &str,
        request: &Value,
    ) -> Result<Option<ProjectTodoRunRecord>> {
        anyhow::ensure!(
            id.starts_with("prun-") && opencoder_core::fleet::valid_id(id),
            "invalid project run id"
        );
        let Some(run) = self.require()?.projects.get_todo_run(id).await? else {
            return Ok(None);
        };
        let saved = crate::trace::input(&run)?;
        anyhow::ensure!(
            run.todo_id == todo_id && run.kind == kind && saved["request"] == *request,
            "run id already accepted with different input"
        );
        Ok(Some(run))
    }
    pub async fn reserve_attempt(
        &self,
        todo_id: &str,
        kind: ProjectTodoRunKind,
        id: &str,
        override_: Option<ExecutorOverride>,
        request: Value,
    ) -> Result<ProjectTodoRunRecord> {
        let deps = self.require()?;
        let _gate = deps.admission.lock().await;
        if let Some(run) = self.accepted_attempt(todo_id, kind, id, &request).await? {
            return Ok(run);
        }
        if let Some(error) = deps.persistence_error.lock().unwrap().as_ref() {
            bail!("project persistence unavailable: {error}");
        }
        let todo = deps
            .projects
            .get_todo(todo_id)
            .await?
            .with_context(|| format!("todo not found: {todo_id}"))?;
        if todo.status == ProjectTodoStatus::Running {
            bail!("todo is running");
        }
        ensure_no_plan_in_flight(&deps, todo_id).await?;
        if kind == ProjectTodoRunKind::Execute && todo.plan_md.is_none() {
            bail!("todo has no plan — generate one first");
        }
        let cx = build_context(&deps, &todo).await?;
        let (resolved, trace) = if kind == ProjectTodoRunKind::Plan {
            (
                ResolvedExecutor {
                    kind: ProjectExecutorKind::Agent,
                    ref_: Some("plan".into()),
                },
                BrainTrace::default(),
            )
        } else if todo.executor_kind == ProjectExecutorKind::Brain {
            resolve_brain(&deps, &todo, &cx, override_.as_ref()).await?
        } else {
            (
                crate::executor::resolve(&todo, override_.as_ref())?,
                BrainTrace::default(),
            )
        };
        let version = deps.projects.next_todo_version(todo_id).await?;
        let now = opencoder_core::message::now_ms();
        let run=ProjectTodoRunRecord{
            id:id.into(),todo_id:todo_id.into(),kind,version,
            plan_md:if kind==ProjectTodoRunKind::Execute{todo.plan_md.clone()}else{None},output_md:None,
            agent:if kind==ProjectTodoRunKind::Plan{"plan".into()}else{run_agent_label(&resolved,&todo)},executor_kind:resolved.kind,
            capability_id:trace.capability_id.clone(),plan_id:trace.plan_id.clone(),output_ref:None,session_id:None,
            status:ProjectTodoRunStatus::Running,started_at:now,finished_at:None,created_at:now,
            input_snapshot:Some(json!({"schema":1,"request":request,"todo":todo,"context":cx,"executor":{"kind":resolved.kind,"ref":resolved.ref_},"override":override_,"resource_root":opencoder_core::agent::scope::current_root()}).to_string()),
            trace_manifest:None,
        };
        anyhow::ensure!(
            deps.projects
                .claim_todo_running_with_run(&run, now)
                .await
                .context("claim todo and create execute run")?,
            "todo is running"
        );
        deps.projects
            .get_todo_run(id)
            .await?
            .context("accepted run disappeared")
    }
    /// Called only after the owner journal has acknowledged the attempt.
    pub async fn drive_reserved(&self, id: &str) -> Result<()> {
        let deps = self.require()?;
        let _gate = deps.admission.lock().await;
        let run = deps
            .projects
            .get_todo_run(id)
            .await?
            .context("reserved run missing")?;
        if run.status != ProjectTodoRunStatus::Running
            || deps.spawns.lock().unwrap().contains_key(id)
        {
            return Ok(());
        }
        let saved = crate::trace::input(&run)?;
        let todo = serde_json::from_value(saved["todo"].clone())?;
        let cx = serde_json::from_value(saved["context"].clone())?;
        let token = spawn_run(&deps, id);
        let run_id = id.to_owned();
        let todo_id = run.todo_id.clone();
        let drive_deps = deps.clone();
        if run.kind == ProjectTodoRunKind::Plan {
            crate::recover::spawn_run_driver(&deps, id, &todo_id, run.kind, move || {
                crate::plan_gen::drive(drive_deps, run_id, todo, cx, token)
            });
        } else {
            let kind = serde_json::from_value(saved["executor"]["kind"].clone())?;
            let resolved = ResolvedExecutor {
                kind,
                ref_: saved["executor"]["ref"].as_str().map(str::to_owned),
            };
            let (resolved, handoff) = if saved["todo"]["executor_kind"] == "brain" {
                let override_ = ExecutorOverride {
                    kind: resolved.kind,
                    ref_: resolved.ref_,
                    capability_id: run.capability_id.clone(),
                    plan_id: run.plan_id.clone(),
                };
                (
                    ResolvedExecutor {
                        kind: ProjectExecutorKind::Brain,
                        ref_: None,
                    },
                    Some(BrainHandoff {
                        override_: Some(override_),
                        trace: Some(BrainTrace {
                            capability_id: run.capability_id,
                            plan_id: run.plan_id,
                        }),
                    }),
                )
            } else {
                (resolved, None)
            };
            crate::recover::spawn_run_driver(&deps, id, &todo_id, run.kind, move || {
                crate::executor::drive(
                    drive_deps,
                    run_id,
                    todo,
                    cx,
                    run.version,
                    resolved,
                    handoff,
                    token,
                )
            });
        }
        Ok(())
    }
}
