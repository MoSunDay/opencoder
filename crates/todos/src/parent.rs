use std::{path::PathBuf, sync::Arc};

use anyhow::{Context, Result};
use opencoder_core::{message::now_ms, resolve_agent, Config, Role};
use opencoder_llm::ChatStream;
use opencoder_store::{SessionMeta, SessionPatch, Store, TASK_TYPE_TODO_WORKFLOW};

use crate::{domain, types::*};

pub struct DecisionRuntime {
    pub store: Arc<dyn Store>,
    pub client: Arc<dyn ChatStream>,
    pub config: Config,
    pub workdir: PathBuf,
}

pub async fn create_session(
    store: &Arc<dyn Store>,
    state: &WorkflowState,
    config: &Config,
) -> Result<()> {
    let now = now_ms();
    store
        .create_session(&SessionMeta {
            id: state.parent_session_id.clone(),
            title: Some(format!("todos workflow {}", state.workflow_id)),
            agent: Some("workflow".into()),
            model: Some(config.model.clone()),
            workdir_hash: None,
            created_at: now,
            updated_at: now,
            summary: None,
            summary_seq: None,
            summary_images: Vec::new(),
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: Some(TASK_TYPE_TODO_WORKFLOW.into()),
            requirement: Some("Manage global TODO state and acceptance".into()),
        })
        .await
}

pub async fn schedule(
    runtime: &DecisionRuntime,
    spec: &WorkflowSpec,
    state: &WorkflowState,
) -> Result<ParentDecision> {
    let runnable = domain::runnable(spec, state);
    let prompt = format!(
        "Decide the next workflow operation. You control how many runnable TODOs execute concurrently. You have no execution authority: never emit or request a tool call, inspect the environment, diagnose credentials, or perform a TODO yourself. Return exactly one raw JSON object with no Markdown. When a runnable TODO is blocked or interrupted, dispatch it with resume or fork, or suspend the workflow; never investigate the blocker yourself.\n\
         Allowed JSON operations:\n\
         {{\"operation\":\"dispatch\",\"todos\":[{{\"todo_id\":\"...\",\"context_mode\":\"new|resume|fork\"}}],\"reason\":\"...\"}}\n\
         {{\"operation\":\"mark_milestone\",\"todo_id\":\"...\",\"reason\":\"...\"}}\n\
         {{\"operation\":\"rewind\",\"milestone_todo_id\":\"...\",\"reason\":\"...\"}}\n\
         {{\"operation\":\"complete|fail|suspend\",\"reason\":\"...\"}}\n\
         Dispatch only IDs in runnable. Use new for first attempt, resume to continue the same interrupted/revision session, fork for a clean attempt. Complete only when every TODO passed.\n\
         RUNNABLE={}\nSTATE={}\nTODO_SUMMARY={}",
        serde_json::to_string(&runnable)?,
        serde_json::to_string(state)?,
        serde_json::to_string(&spec.todos.iter().map(|t| serde_json::json!({"id":t.id,"title":t.title,"depends_on":t.depends_on})).collect::<Vec<_>>())?
    );
    decide(runtime, state, prompt).await
}

pub async fn accept(
    runtime: &DecisionRuntime,
    spec: &WorkflowSpec,
    state: &WorkflowState,
    todo_id: &str,
    gate: &serde_json::Value,
) -> Result<AcceptanceDecision> {
    let todo = spec
        .todos
        .iter()
        .find(|todo| todo.id == todo_id)
        .context("acceptance TODO not found")?;
    let candidate = state.todos[todo_id]
        .candidate
        .as_ref()
        .context("candidate missing")?;
    let prompt = format!(
        "Accept or reject one TODO candidate. You have no execution authority: never emit or request a tool call, inspect the environment, diagnose credentials, or perform the TODO yourself. Return exactly one raw JSON object with no Markdown. Required tool gates are authoritative: when gate.ok=false you MUST revise or fail.\n\
         Allowed JSON operations:\n\
         {{\"operation\":\"accept\",\"reason\":\"...\",\"mark_milestone\":false}}\n\
         {{\"operation\":\"revise\",\"reason\":\"...\",\"context_mode\":\"resume|fork\"}}\n\
         {{\"operation\":\"rewind\",\"milestone_todo_id\":\"...\",\"reason\":\"...\"}}\n\
         {{\"operation\":\"fail\",\"reason\":\"...\"}}\n\
         TODO={}\nCANDIDATE={}\nTOOL_GATE={}\nMILESTONES={}",
        serde_json::to_string(todo)?,
        serde_json::to_string(candidate)?,
        serde_json::to_string(gate)?,
        serde_json::to_string(&state.milestones)?
    );
    decide(runtime, state, prompt).await
}

async fn decide<T: serde::de::DeserializeOwned>(
    runtime: &DecisionRuntime,
    state: &WorkflowState,
    prompt: String,
) -> Result<T> {
    let mut config = runtime.config.clone();
    config.autopilot.enabled = false;
    let mut session = opencoder_session::resume(
        runtime.store.clone(),
        &state.parent_session_id,
        config,
        runtime.client.clone(),
        runtime.workdir.clone(),
    )
    .await?;
    session.agent = resolve_agent("workflow").context("workflow agent not registered")?;
    opencoder_session::run(&mut session, prompt, |_| {}).await?;
    let raw = session
        .messages
        .iter()
        .rev()
        .find(|message| message.role == Role::Assistant)
        .map(|message| message.text())
        .context("workflow agent returned no assistant decision")?;
    let decision = crate::json_output::parse(&raw)
        .with_context(|| format!("workflow agent returned invalid JSON: {raw}"))?;
    let seq = runtime
        .store
        .last_message_seq(&state.parent_session_id)
        .await?;
    runtime
        .store
        .update_session(
            &state.parent_session_id,
            &SessionPatch {
                summary: Some(serde_json::to_string(state)?),
                summary_seq: Some(seq),
                updated_at: Some(now_ms()),
                ..Default::default()
            },
        )
        .await?;
    Ok(decision)
}
