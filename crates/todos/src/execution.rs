use std::{collections::HashMap, path::Path, sync::Arc};

use anyhow::{Context, Result};
use opencoder_core::{message::now_ms, resolve_agent, Config, Role};
use opencoder_llm::ChatStream;
use opencoder_session::SessionEvent;
use opencoder_store::{SessionMeta, Store, TASK_TYPE_TODO};
use tokio_util::sync::CancellationToken;

use crate::{domain::json_contains, types::*};

pub struct TodoExecution {
    pub candidate: Candidate,
    pub gate: serde_json::Value,
}

#[allow(clippy::too_many_arguments)]
pub async fn execute(
    store: Arc<dyn Store>,
    client: Arc<dyn ChatStream>,
    mut config: Config,
    workdir: &Path,
    workflow: &WorkflowSpec,
    state: &WorkflowState,
    todo: &TodoSpec,
    context_mode: ContextMode,
    session_id: String,
    cancel: CancellationToken,
) -> Result<TodoExecution> {
    config.autopilot.enabled = false;
    let agent = resolve_agent(&todo.agent)
        .with_context(|| format!("TODO {} has unknown agent {}", todo.id, todo.agent))?;
    if !agent.is_primary() || agent.name == "workflow" {
        anyhow::bail!(
            "TODO {} agent must be a non-workflow primary agent",
            todo.id
        );
    }
    let mut session = if context_mode == ContextMode::Resume {
        let existing = state.todos[&todo.id]
            .active_session_id
            .as_deref()
            .context("resume requested without active session")?;
        opencoder_session::resume(
            store.clone(),
            existing,
            config,
            client,
            workdir.to_path_buf(),
        )
        .await?
    } else {
        prepare_session(&store, workflow, todo, &session_id, &config).await?;
        opencoder_session::SessionState::new(
            session_id,
            agent,
            config,
            client,
            workdir.to_path_buf(),
        )
        .with_store(store.clone())
        .mark_session_created()
    };
    session.cancel = Some(cancel);
    let prompt = focused_prompt(workflow, state, todo, context_mode)?;
    let event_seq = store.last_event_seq(&session.id).await?;
    opencoder_session::run(&mut session, prompt, |_| {}).await?;
    let events = store
        .events_after(&session.id, event_seq)
        .await?
        .into_iter()
        .map(|record| {
            let kind = record
                .sse_kind
                .as_deref()
                .context("TODO event is missing its exact SSE kind")?;
            SessionEvent::from_sse(kind, record.payload)
                .with_context(|| format!("decode persisted TODO event {kind}"))
        })
        .collect::<Result<Vec<_>>>()?;
    let raw = session
        .messages
        .iter()
        .rev()
        .find(|message| message.role == Role::Assistant)
        .map(|message| message.text())
        .context("TODO agent returned no final candidate")?;
    let candidate = parse_candidate(&raw)
        .with_context(|| format!("TODO {} returned invalid candidate JSON: {raw}", todo.id))?;
    let gate = evaluate_gate(todo, &events);
    Ok(TodoExecution { candidate, gate })
}

fn parse_candidate(raw: &str) -> Result<Candidate> {
    crate::json_output::parse(raw)
}

pub async fn prepare_session(
    store: &Arc<dyn Store>,
    workflow: &WorkflowSpec,
    todo: &TodoSpec,
    session_id: &str,
    config: &Config,
) -> Result<()> {
    let now = now_ms();
    store
        .create_session(&SessionMeta {
            id: session_id.into(),
            title: Some(format!("{} / {}", workflow.name, todo.title)),
            agent: Some(todo.agent.clone()),
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
            task_type: Some(TASK_TYPE_TODO.into()),
            requirement: Some(todo.instructions.clone()),
        })
        .await
}

fn focused_prompt(
    workflow: &WorkflowSpec,
    state: &WorkflowState,
    todo: &TodoSpec,
    context_mode: ContextMode,
) -> Result<String> {
    let dependencies = todo
        .depends_on
        .iter()
        .map(|id| {
            let item = &state.todos[id];
            serde_json::json!({
                "todo_id": id,
                "summary": item.candidate.as_ref().map(|candidate| &candidate.summary),
                "evidence_refs": item.candidate.as_ref().map(|candidate| &candidate.evidence_refs),
            })
        })
        .collect::<Vec<_>>();
    let recovery = state.todos[&todo.id]
        .candidate
        .as_ref()
        .map(|candidate| &candidate.recovery_context);
    Ok(format!(
        "Complete exactly one focused TODO. You may use available tools and may delegate supporting work through the task tool, but must not advance another TODO. Return only the final Candidate JSON object with fields status(candidate|blocked|interrupted), summary(string), result(string|null), verification(string), evidence_refs(string[]), recovery_context{{summary:string,refs:string[]}}.\n\
         WORKFLOW_OBJECTIVE={}\nCONSTRAINTS={}\nTODO={}\nACCEPTED_DEPENDENCIES={}\nCONTEXT_MODE={}\nPREVIOUS_RECOVERY={}",
        serde_json::to_string(&workflow.objective)?,
        serde_json::to_string(&workflow.constraints)?,
        serde_json::to_string(todo)?,
        serde_json::to_string(&dependencies)?,
        serde_json::to_string(&context_mode)?,
        serde_json::to_string(&recovery)?,
    ))
}

fn evaluate_gate(todo: &TodoSpec, events: &[SessionEvent]) -> serde_json::Value {
    let starts: HashMap<&str, (&str, &serde_json::Value)> = events
        .iter()
        .filter_map(|event| match event {
            SessionEvent::ToolStart { id, name, input } => {
                Some((id.as_str(), (name.as_str(), input)))
            }
            _ => None,
        })
        .collect();
    let ends: HashMap<&str, bool> = events
        .iter()
        .filter_map(|event| match event {
            SessionEvent::ToolEnd { id, is_error, .. } => Some((id.as_str(), !*is_error)),
            _ => None,
        })
        .collect();
    let checks = todo
        .acceptance
        .required_tool_calls
        .iter()
        .map(|required| {
            let matched = starts.iter().any(|(id, (name, input))| {
                *name == required.name
                    && json_contains(input, &required.arguments_contains)
                    && ends.get(id).copied() == Some(required.result_ok)
            });
            serde_json::json!({"required":required,"matched":matched})
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "ok": checks.iter().all(|check| check["matched"] == true),
        "checks": checks
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLOCKED_CANDIDATE: &str = r#"{"status":"blocked","summary":"blocked","result":"none","verification":"failed","evidence_refs":[],"recovery_context":{"summary":"retry later","refs":[]}}"#;

    #[test]
    fn candidate_parser_accepts_raw_and_single_fenced_json() {
        for value in [
            BLOCKED_CANDIDATE.to_string(),
            format!("```json\n{BLOCKED_CANDIDATE}\n```"),
            format!("```\n{BLOCKED_CANDIDATE}\n```"),
        ] {
            assert_eq!(
                parse_candidate(&value).unwrap().status,
                CandidateStatus::Blocked
            );
        }
    }

    #[test]
    fn candidate_parser_rejects_explanatory_text_around_json() {
        assert!(parse_candidate(&format!("result:\n{BLOCKED_CANDIDATE}")).is_err());
    }

    #[test]
    fn blocked_candidate_may_have_no_result() {
        let value = BLOCKED_CANDIDATE.replace("\"none\"", "null");
        assert_eq!(parse_candidate(&value).unwrap().result, None);
    }

    #[test]
    fn gate_requires_matching_start_and_successful_end() {
        let todo = TodoSpec {
            id: "x".into(),
            title: "x".into(),
            requirement_background: "x".into(),
            instructions: "x".into(),
            depends_on: vec![],
            agent: "act".into(),
            max_attempts: 1,
            acceptance: AcceptanceSpec {
                criteria: "x".into(),
                required_tool_calls: vec![RequiredToolCall {
                    name: "mcp__fk__tap".into(),
                    arguments_contains: serde_json::json!({"label":"A"}),
                    result_ok: true,
                }],
            },
            metadata: serde_json::Value::Null,
        };
        let events = vec![
            SessionEvent::ToolStart {
                id: "1".into(),
                name: "mcp__fk__tap".into(),
                input: serde_json::json!({"label":"A","x":1}),
            },
            SessionEvent::ToolEnd {
                id: "1".into(),
                name: "mcp__fk__tap".into(),
                output: "ok".into(),
                is_error: false,
                images: vec![],
            },
        ];
        assert_eq!(evaluate_gate(&todo, &events)["ok"], true);
    }
}
