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
    correction: Option<&str>,
) -> Result<ParentDecision> {
    let runnable = domain::runnable(spec, state)?;
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
    // `None` keeps the prompt byte-identical to the pre-correction form.
    let prompt = match correction {
        None => prompt,
        Some(note) => format!("{prompt}\nCORRECTION: {note}"),
    };
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
    let candidate = state
        .todos
        .get(todo_id)
        .with_context(|| format!("state missing TODO {todo_id}"))?
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

/// Maximum correction re-asks when the workflow agent replies with something
/// that is not a single parsable JSON object: 1 initial ask + 2 corrections.
const PARSE_RETRIES: u32 = 2;

async fn decide<T: serde::de::DeserializeOwned>(
    runtime: &DecisionRuntime,
    state: &WorkflowState,
    prompt: String,
) -> Result<T> {
    let mut config = runtime.config.clone();
    config.autopilot.mode = opencoder_core::ApMode::Off;
    let mut session = opencoder_session::resume(
        runtime.store.clone(),
        &state.parent_session_id,
        config,
        runtime.client.clone(),
        runtime.workdir.clone(),
    )
    .await?;
    session.agent = resolve_agent("workflow").context("workflow agent not registered")?;
    // Bug #16b: one unparseable reply must not suspend the whole workflow.
    // Re-ask in the same session with a correction prompt, bounded retries.
    // Only assistant messages produced by each ask count as its answer — the
    // parent transcript keeps earlier decision JSON that must never be
    // recycled as a fresh reply (same watermark rule as execution.rs).
    let mut watermark = session.messages.len();
    let mut retries_left = PARSE_RETRIES;
    let mut prompt = prompt;
    let decision = loop {
        opencoder_session::run(&mut session, prompt, |_| {}).await?;
        let raw = session
            .messages
            .iter()
            .skip(watermark)
            .rev()
            .find(|message| message.role == Role::Assistant)
            .map(|message| message.text())
            .context("workflow agent returned no assistant decision")?;
        watermark = session.messages.len();
        match crate::json_output::parse(&raw) {
            Ok(decision) => break decision,
            Err(error) if retries_left > 0 => {
                retries_left -= 1;
                let reason = format!("{error:#}");
                tracing::info!(
                    session_id = %state.parent_session_id,
                    error = %reason,
                    retries_left,
                    "workflow agent reply was not parsable JSON; re-asking with correction"
                );
                prompt = format!(
                    "Your previous reply could not be parsed as the required JSON object ({reason}). Re-send the decision now: exactly one raw JSON object, no Markdown fences and no explanation text, matching the allowed operations from the previous message."
                );
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("workflow agent returned invalid JSON: {raw}"));
            }
        }
    };
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
