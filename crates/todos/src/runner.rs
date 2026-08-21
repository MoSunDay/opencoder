use std::{path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use opencoder_core::Config;
use opencoder_llm::ChatStream;
use opencoder_store::Store;
use tokio_util::sync::CancellationToken;

use crate::{batch, domain, parent, persistence, transitions, types::*};

/// Maximum decision-correction re-asks in `drive_inner`: 1 initial decision
/// plus 2 corrected re-decisions before the invalid decision bails.
const DISPATCH_CORRECTION_RETRIES: u32 = 2;

/// Bounded retries for `interrupt`'s load→mutate→commit loop: the driving
/// process bumps the generation on every transition, so a commit taken while
/// the workflow is actively Running loses the optimistic-concurrency race
/// with high probability. Reloading and retrying is correct because every
/// iteration re-derives the terminal-state check from the freshest state.
const INTERRUPT_COMMIT_RETRIES: u32 = 3;

#[derive(Clone)]
pub struct Runtime {
    pub store: Arc<dyn Store>,
    pub client: Arc<dyn ChatStream>,
    pub config: Config,
    pub workdir: PathBuf,
    pub debug_root: Option<PathBuf>,
    pub cancel: CancellationToken,
}

impl Runtime {
    pub async fn run_new(&self, spec: WorkflowSpec) -> Result<WorkflowState> {
        let workflow_id = format!("todos-{}", ulid::Ulid::new());
        self.run_new_with_id(spec, workflow_id).await
    }

    pub async fn run_new_with_id(
        &self,
        spec: WorkflowSpec,
        workflow_id: String,
    ) -> Result<WorkflowState> {
        domain::validate_spec(&spec)?;
        if self.store.get_todo_workflow(&workflow_id).await?.is_some() {
            anyhow::bail!("todo workflow already exists: {workflow_id}");
        }
        let parent_id = format!("todo-workflow-{}", ulid::Ulid::new());
        let mut state = domain::initial_state(&spec, workflow_id, parent_id);
        parent::create_session(&self.store, &state, &self.config).await?;
        persistence::create(&self.store, &spec, &state).await?;
        self.dump(&spec, &state).await?;
        state.status = WorkflowStatus::Running;
        state.generation += 1;
        self.commit(&spec, &state, "workflow_started", serde_json::json!({}))
            .await?;
        self.drive(spec, state).await
    }

    pub async fn resume(&self, workflow_id: &str) -> Result<WorkflowState> {
        let (spec, state) = persistence::load(&self.store, workflow_id)
            .await?
            .with_context(|| format!("todo workflow not found: {workflow_id}"))?;
        domain::validate_spec(&spec)?;
        // Bug #16c: a persisted Running status means another runner is
        // driving this workflow (or a previous runner crashed mid-flight and
        // the state never converged). Two drivers would fight over the
        // generation CAS, so resume refuses until an interrupt has parked
        // the workflow — `opencoder todos interrupt <id>` is the takeover
        // path after a crash.
        if state.status == WorkflowStatus::Running {
            anyhow::bail!(
                "todo workflow {workflow_id} is still running: another runner may be driving it. \
                 Run `opencoder todos interrupt {workflow_id}` first, then resume to take over"
            );
        }
        if matches!(
            state.status,
            WorkflowStatus::Completed | WorkflowStatus::Failed
        ) {
            return Ok(state);
        }
        let mut state = transitions::reconcile_interrupted(state);
        // Persist Running at the resume boundary (mirrors run_new's
        // workflow_started): leaving the stored status Pending/Suspended
        // until the first dispatch misleads observers and opens a window
        // where a second driver sees "not running" during the decision phase.
        // No extra generation bump: reconcile_interrupted already provides
        // the single +1 the CAS commit below expects.
        if state.status != WorkflowStatus::Running {
            state.status = WorkflowStatus::Running;
        }
        self.commit(
            &spec,
            &state,
            "workflow_resumed",
            serde_json::json!({"reconciled_active_todos":true}),
        )
        .await?;
        self.drive(spec, state).await
    }

    async fn drive(&self, spec: WorkflowSpec, mut state: WorkflowState) -> Result<WorkflowState> {
        let result = self.drive_inner(&spec, &mut state).await;
        if let Err(error) = result {
            // Takeover probe: has an external writer (interrupt / another
            // runner) already parked this workflow? The probe must never
            // mask the runtime error it is diagnosing — a store failure
            // here degrades to a warn + no takeover check, the local
            // suspension path below still runs and the original error keeps
            // propagating.
            let latest = match persistence::load(&self.store, &state.workflow_id).await {
                Ok(latest) => latest,
                Err(probe_error) => {
                    tracing::warn!(
                        workflow_id = %state.workflow_id,
                        error = %format!("{probe_error:#}"),
                        "todo workflow takeover probe failed; continuing with local suspension"
                    );
                    None
                }
            };
            if let Some((_, latest)) = latest {
                if latest.generation != state.generation
                    && latest.status == WorkflowStatus::Suspended
                {
                    tracing::info!(
                        workflow_id = %state.workflow_id,
                        local_generation = state.generation,
                        remote_generation = latest.generation,
                        "todo workflow takeover detected: adopting external suspended state"
                    );
                    return Ok(latest);
                }
            }
            if !matches!(
                state.status,
                WorkflowStatus::Completed | WorkflowStatus::Failed
            ) {
                let suspended = transitions::terminal(
                    state.clone(),
                    WorkflowStatus::Suspended,
                    format!("{error:#}"),
                )?;
                self.commit(
                    &spec,
                    &suspended,
                    "runtime_error",
                    serde_json::json!({"error":format!("{error:#}")}),
                )
                .await
                .with_context(|| {
                    format!("runtime failed and suspension could not be persisted: {error:#}")
                })?;
                tracing::info!(
                    workflow_id = %state.workflow_id,
                    error = %format!("{error:#}"),
                    "todo workflow suspended after runtime error"
                );
            }
            return Err(error);
        }
        Ok(state)
    }

    async fn drive_inner(&self, spec: &WorkflowSpec, state: &mut WorkflowState) -> Result<()> {
        let parent_runtime = self.parent_runtime();
        for _cycle in 0..1000 {
            if self.cancel.is_cancelled() {
                // An external writer (e.g. `runner::interrupt` from another
                // process) may have already moved the persisted generation —
                // most importantly a Suspended verdict. Adopt that state
                // instead of committing a local "workflow_interrupted" over
                // it. A local cancel keeps the store generation in sync with
                // the in-memory state, so it falls through to the original
                // local-interrupt path below.
                let external = persistence::load(&self.store, &state.workflow_id)
                    .await
                    .ok()
                    .flatten()
                    .filter(|(_, latest)| latest.generation != state.generation);
                if let Some((_, latest)) = external {
                    tracing::info!(
                        workflow_id = %state.workflow_id,
                        generation = latest.generation,
                        "local interrupt superseded by externally advanced workflow state"
                    );
                    *state = latest;
                    return Ok(());
                }
                *state = transitions::terminal(
                    state.clone(),
                    WorkflowStatus::Suspended,
                    "local interrupt requested".into(),
                )?;
                self.commit(spec, state, "workflow_interrupted", serde_json::json!({}))
                    .await?;
                return Ok(());
            }
            let mut correction: Option<String> = None;
            let mut retries_left = DISPATCH_CORRECTION_RETRIES;
            let decision = loop {
                let decision =
                    parent::schedule(&parent_runtime, spec, state, correction.as_deref()).await?;
                // Bug #16b/M6: an invalid model decision (unrunnable id,
                // wrong context_mode, exhausted attempts, non-milestone
                // rewind, completing with unaccepted TODOs, ...) is a
                // correctable model mistake, not a runtime failure — for
                // EVERY operation, not just dispatches: dry-run the dispatch
                // rules or the actual transition on a throwaway clone and
                // re-ask the parent with a correction before a bad decision
                // suspends the whole workflow.
                match batch::validate_decision(spec, state, &decision) {
                    Ok(()) => break decision,
                    Err(error) if retries_left > 0 => {
                        retries_left -= 1;
                        let reason = format!("{error:#}");
                        tracing::info!(
                            workflow_id = %state.workflow_id,
                            error = %reason,
                            retries_left,
                            "parent decision rejected; re-asking with correction"
                        );
                        correction = Some(format!(
                            "your previous decision was rejected: {reason}. Dispatch only runnable TODO ids with a valid context_mode, or choose another allowed operation."
                        ));
                    }
                    Err(error) => {
                        return Err(error
                            .context("parent decision stayed invalid after correction retries"));
                    }
                }
            };
            match decision {
                ParentDecision::Dispatch { todos, reason } => {
                    batch::execute(self, spec, state, todos, reason).await?;
                }
                ParentDecision::MarkMilestone { todo_id, reason } => {
                    // validate_decision already proved the todo is Passed.
                    // Re-marking an existing milestone is an idempotent no-op:
                    // rewind-recovery flows legitimately re-mark it after
                    // re-acceptance, mirroring the silent insert in
                    // accepted(mark_milestone=true). One bail here used to
                    // suspend the whole workflow.
                    if !state.milestones.insert(todo_id.clone()) {
                        tracing::info!(
                            workflow_id = %state.workflow_id,
                            todo_id = %todo_id,
                            "milestone re-marked; treating as idempotent no-op"
                        );
                        continue;
                    }
                    state.generation += 1;
                    self.commit(
                        spec,
                        state,
                        "milestone_marked",
                        serde_json::json!({"todo_id":todo_id,"reason":reason}),
                    )
                    .await?;
                }
                ParentDecision::Rewind {
                    milestone_todo_id,
                    reason,
                } => {
                    *state = transitions::rewind(
                        spec,
                        state.clone(),
                        &milestone_todo_id,
                        reason.clone(),
                    )?;
                    self.commit(
                        spec,
                        state,
                        "workflow_rewound",
                        serde_json::json!({"milestone_todo_id":milestone_todo_id,"reason":reason}),
                    )
                    .await?;
                }
                ParentDecision::Complete { reason } => {
                    *state = transitions::terminal(
                        state.clone(),
                        WorkflowStatus::Completed,
                        reason.clone(),
                    )?;
                    self.commit(
                        spec,
                        state,
                        "workflow_completed",
                        serde_json::json!({"reason":reason}),
                    )
                    .await?;
                    return Ok(());
                }
                ParentDecision::Fail { reason } => {
                    *state = transitions::terminal(
                        state.clone(),
                        WorkflowStatus::Failed,
                        reason.clone(),
                    )?;
                    self.commit(
                        spec,
                        state,
                        "workflow_failed",
                        serde_json::json!({"reason":reason}),
                    )
                    .await?;
                    return Ok(());
                }
                ParentDecision::Suspend { reason } => {
                    *state = transitions::terminal(
                        state.clone(),
                        WorkflowStatus::Suspended,
                        reason.clone(),
                    )?;
                    self.commit(
                        spec,
                        state,
                        "workflow_suspended",
                        serde_json::json!({"reason":reason}),
                    )
                    .await?;
                    return Ok(());
                }
            }
        }
        anyhow::bail!("workflow exceeded 1000 parent decisions")
    }

    pub(crate) async fn commit(
        &self,
        spec: &WorkflowSpec,
        state: &WorkflowState,
        kind: &str,
        payload: serde_json::Value,
    ) -> Result<()> {
        persistence::commit(&self.store, spec, state, kind, payload).await?;
        self.dump(spec, state).await
    }

    async fn dump(&self, spec: &WorkflowSpec, state: &WorkflowState) -> Result<()> {
        if let Some(root) = &self.debug_root {
            if let Err(error) = persistence::debug_dump(&self.store, spec, state, root).await {
                tracing::warn!(
                    workflow_id = %state.workflow_id,
                    error = %error,
                    "debug dump failed; continuing without projection refresh"
                );
            }
        }
        Ok(())
    }

    pub(crate) fn parent_runtime(&self) -> parent::DecisionRuntime {
        parent::DecisionRuntime {
            store: self.store.clone(),
            client: self.client.clone(),
            config: self.config.clone(),
            workdir: self.workdir.clone(),
        }
    }
}

pub(crate) async fn poll_interrupt(
    store: Arc<dyn Store>,
    workflow_id: String,
    generation: u64,
    cancel: CancellationToken,
) {
    loop {
        tokio::time::sleep(Duration::from_millis(250)).await;
        if cancel.is_cancelled() {
            return;
        }
        match store.get_todo_workflow(&workflow_id).await {
            Ok(Some(record))
                if record.generation != generation as i64 || record.status == "suspended" =>
            {
                cancel.cancel();
                return;
            }
            Ok(Some(_)) => {}
            Ok(None) => {
                cancel.cancel();
                return;
            }
            Err(error) => {
                tracing::warn!(
                    workflow_id = %workflow_id,
                    error = %error,
                    "transient store error while polling todo workflow interrupt state"
                );
            }
        }
    }
}

pub async fn interrupt(
    store: &Arc<dyn Store>,
    workflow_id: &str,
    reason: &str,
) -> Result<WorkflowState> {
    // Load→mutate→CAS-commit with bounded retries: interrupt is typically
    // called exactly while another process is driving the workflow, and that
    // driver bumps the generation on every transition — a single-shot commit
    // spuriously fails the optimistic-concurrency check. On a generation
    // conflict, reload the freshest state and retry (the terminal-state
    // refusal is re-derived on every iteration).
    let mut retries_left = INTERRUPT_COMMIT_RETRIES;
    loop {
        let (spec, state) = persistence::load(store, workflow_id)
            .await?
            .with_context(|| format!("todo workflow not found: {workflow_id}"))?;
        if matches!(
            state.status,
            WorkflowStatus::Completed | WorkflowStatus::Failed
        ) {
            anyhow::bail!("cannot interrupt terminal workflow {workflow_id}");
        }
        let next = transitions::terminal(state, WorkflowStatus::Suspended, reason.into())?;
        match persistence::commit(
            store,
            &spec,
            &next,
            "workflow_interrupted",
            serde_json::json!({"reason":reason}),
        )
        .await
        {
            Ok(_) => {
                tracing::info!(
                    workflow_id = %workflow_id,
                    reason = %reason,
                    "todo workflow interrupted"
                );
                return Ok(next);
            }
            Err(error)
                if retries_left > 0 && format!("{error:#}").contains("generation conflict") =>
            {
                retries_left -= 1;
                tracing::info!(
                    workflow_id = %workflow_id,
                    retries_left,
                    "interrupt commit lost the generation race; reloading and retrying"
                );
            }
            Err(error) => return Err(error),
        }
    }
}
