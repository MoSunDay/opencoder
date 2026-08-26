use std::{collections::HashMap, path::Path, sync::Arc};

use anyhow::{Context, Result};
use opencoder_core::{
    message::now_ms, resolve_agent, Config, ContentBlock, Message, Role, ToolFilter,
};
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
    config.autopilot.mode = opencoder_core::ApMode::Off;
    let mut agent = resolve_agent(&todo.agent)
        .with_context(|| format!("TODO {} has unknown agent {}", todo.id, todo.agent))?;
    if !agent.is_primary() || agent.name == "workflow" {
        anyhow::bail!(
            "TODO {} agent must be a non-workflow primary agent",
            todo.id
        );
    }
    agent.tools = ToolFilter::Allow(if workflow.schema_version >= 2 {
        todo.allowed_tools.clone()
    } else {
        allowed_tool_names(todo)
    });
    let mut session = if context_mode == ContextMode::Resume {
        let existing = state
            .todos
            .get(&todo.id)
            .with_context(|| format!("state missing TODO {}", todo.id))?
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
            agent.clone(),
            config,
            client,
            workdir.to_path_buf(),
        )
        .with_store(store.clone())
        .mark_session_created()
    };
    // Resume restores a historical agent snapshot; the current TodoSpec is
    // authoritative for the focused tool boundary.
    session.agent = agent;
    let completion_cancel = cancel.child_token();
    session.cancel = Some(completion_cancel.clone());
    // Snapshot the transcript size before this run: on Resume the session
    // carries the previous attempt's messages, and only assistant messages
    // produced by THIS run are valid candidates.
    let watermark = session.messages.len();
    let prompt = focused_prompt(workflow, state, todo, context_mode)?;
    let message_seq = store.last_message_seq(&session.id).await?;
    let mut completion_gate = CompletionGate::new(workflow.schema_version, todo);
    opencoder_session::run(&mut session, prompt, |event| {
        if completion_gate.observe(&event) {
            completion_cancel.cancel();
        }
    })
    .await?;
    let invocation_messages = store.load_messages_after(&session.id, message_seq).await?;
    let gate = evaluate_gate(workflow.schema_version, todo, &invocation_messages);
    let raw = latest_new_assistant(&session.messages, watermark);
    if cancel.is_cancelled() && !completion_gate.completed {
        anyhow::bail!("TODO execution was interrupted");
    }
    let candidate = resolve_candidate(
        workflow.schema_version,
        todo,
        raw.as_deref(),
        &gate,
        completion_gate.completed,
    )
    .with_context(|| match raw {
        Some(raw) => format!("TODO {} returned invalid candidate JSON: {raw}", todo.id),
        None => format!("TODO {} returned no final candidate", todo.id),
    })?;
    Ok(TodoExecution { candidate, gate })
}

fn allowed_tool_names(todo: &TodoSpec) -> Vec<String> {
    let mut names = todo
        .acceptance
        .required_tool_calls
        .iter()
        .map(|call| call.name.clone())
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names
}

/// Latest assistant text among messages appended after `watermark` (the
/// message-count snapshot taken before the run). Resume-mode sessions keep
/// the previous attempt's assistant transcript, so the watermark prevents a
/// stale candidate from being recycled when the current run produced no new
/// assistant message.
fn latest_new_assistant(messages: &[Message], watermark: usize) -> Option<String> {
    messages
        .iter()
        .skip(watermark)
        .rev()
        .find(|message| message.role == Role::Assistant)
        .map(|message| message.text())
}

fn parse_candidate(raw: &str) -> Result<Candidate> {
    crate::json_output::parse(raw)
}

fn resolve_candidate(
    schema_version: u32,
    todo: &TodoSpec,
    raw: Option<&str>,
    gate: &serde_json::Value,
    runtime_completed_gate: bool,
) -> Result<Candidate> {
    match raw.map(parse_candidate).transpose() {
        Ok(Some(candidate)) => {
            if runtime_completed_gate
                && matches!(
                    candidate.status,
                    CandidateStatus::Blocked | CandidateStatus::Interrupted
                )
                && deterministic_candidate_allowed(schema_version, todo, gate)
            {
                Ok(deterministic_candidate(todo))
            } else {
                Ok(candidate)
            }
        }
        Ok(None) | Err(_) if deterministic_candidate_allowed(schema_version, todo, gate) => {
            Ok(deterministic_candidate(todo))
        }
        Ok(None) => anyhow::bail!("TODO agent returned no final candidate"),
        Err(error) => Err(error),
    }
}

fn deterministic_candidate(todo: &TodoSpec) -> Candidate {
    Candidate {
        status: CandidateStatus::Candidate,
        summary: format!(
            "Completed {}; every declared required tool call succeeded in order.",
            todo.title
        ),
        result: Some(todo.acceptance.criteria.clone()),
        verification: "The schema-v2 runtime verified every required tool call and successful result in declaration order.".into(),
        evidence_refs: Vec::new(),
        recovery_context: RecoveryContext {
            summary: "No recovery required; the runtime derived this candidate from the completed hard tool gate.".into(),
            refs: Vec::new(),
        },
    }
}

fn deterministic_candidate_allowed(
    schema_version: u32,
    todo: &TodoSpec,
    gate: &serde_json::Value,
) -> bool {
    schema_version >= 2
        && !todo.acceptance.required_tool_calls.is_empty()
        && gate.get("ok").and_then(serde_json::Value::as_bool) == Some(true)
}

#[derive(Clone)]
struct ObservedCall {
    id: String,
    name: String,
    input: serde_json::Value,
    ok: Option<bool>,
}

struct CompletionGate<'a> {
    required: &'a [RequiredToolCall],
    calls: Vec<ObservedCall>,
    enabled: bool,
    completed: bool,
}

impl<'a> CompletionGate<'a> {
    fn new(schema_version: u32, todo: &'a TodoSpec) -> Self {
        Self {
            required: &todo.acceptance.required_tool_calls,
            calls: Vec::new(),
            enabled: schema_version >= 2 && !todo.acceptance.required_tool_calls.is_empty(),
            completed: false,
        }
    }

    fn observe(&mut self, event: &SessionEvent) -> bool {
        if !self.enabled || self.completed {
            return self.completed;
        }
        match event {
            SessionEvent::ToolStart { id, name, input } => self.calls.push(ObservedCall {
                id: id.clone(),
                name: name.clone(),
                input: input.clone(),
                ok: None,
            }),
            SessionEvent::ToolEnd { id, is_error, .. } => {
                if let Some(call) = self.calls.iter_mut().find(|call| call.id == *id) {
                    call.ok = Some(!*is_error);
                }
            }
            _ => {}
        }
        self.completed = ordered_required_calls_match(&self.calls, self.required);
        self.completed
    }
}

fn ordered_required_calls_match(calls: &[ObservedCall], required: &[RequiredToolCall]) -> bool {
    required.len() <= calls.len()
        && calls
            .windows(required.len())
            .any(|window| window.iter().zip(required).all(observed_call_matches))
}

fn observed_call_matches((call, required): (&ObservedCall, &RequiredToolCall)) -> bool {
    call.name == required.name
        && json_contains(&call.input, &required.arguments_contains)
        && call.ok == Some(required.result_ok)
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

            autopilot_mode: None,
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
            plan_snapshot: None,
            plan_input_count: 0,
        })
        .await
}

fn focused_prompt(
    workflow: &WorkflowSpec,
    state: &WorkflowState,
    todo: &TodoSpec,
    context_mode: ContextMode,
) -> Result<String> {
    let mut dependencies = Vec::new();
    for id in &todo.depends_on {
        let item = state
            .todos
            .get(id)
            .with_context(|| format!("state missing TODO {id}"))?;
        dependencies.push(serde_json::json!({
            "todo_id": id,
            "summary": item.candidate.as_ref().map(|candidate| &candidate.summary),
            "evidence_refs": item.candidate.as_ref().map(|candidate| &candidate.evidence_refs),
        }));
    }
    let recovery = state
        .todos
        .get(&todo.id)
        .with_context(|| format!("state missing TODO {}", todo.id))?
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

#[derive(Clone, Copy)]
struct TranscriptCall<'a> {
    name: &'a str,
    input: &'a serde_json::Value,
    ok: Option<bool>,
}

fn transcript_calls(messages: &[Message]) -> Vec<TranscriptCall<'_>> {
    let results: HashMap<&str, bool> = messages
        .iter()
        .flat_map(|message| &message.blocks)
        .filter_map(|block| match block {
            ContentBlock::ToolResult {
                tool_use_id,
                is_error,
                ..
            } => Some((tool_use_id.as_str(), !*is_error)),
            _ => None,
        })
        .collect();
    messages
        .iter()
        .flat_map(|message| &message.blocks)
        .filter_map(|block| match block {
            ContentBlock::ToolUse { id, name, input } => Some(TranscriptCall {
                name,
                input,
                ok: results.get(id.as_str()).copied(),
            }),
            _ => None,
        })
        .collect()
}

fn call_matches(call: TranscriptCall<'_>, required: &RequiredToolCall) -> bool {
    call.name == required.name
        && json_contains(call.input, &required.arguments_contains)
        && call.ok == Some(required.result_ok)
}

fn evaluate_gate(schema_version: u32, todo: &TodoSpec, messages: &[Message]) -> serde_json::Value {
    let calls = transcript_calls(messages);
    if schema_version >= 2 {
        let required = &todo.acceptance.required_tool_calls;
        let matched = !required.is_empty()
            && required.len() <= calls.len()
            && calls.windows(required.len()).any(|window| {
                window
                    .iter()
                    .copied()
                    .zip(required)
                    .all(|(call, required)| call_matches(call, required))
            });
        let check_matches = best_ordered_check_matches(&calls, required);
        return serde_json::json!({
            "ok": matched,
            "checks": required
                .iter()
                .zip(check_matches)
                .map(|(required, matched)| serde_json::json!({"required":required,"matched":matched}))
                .collect::<Vec<_>>()
        });
    }
    let checks = todo
        .acceptance
        .required_tool_calls
        .iter()
        .map(|required| {
            let index = calls.iter().position(|call| call_matches(*call, required));
            serde_json::json!({"required":required,"matched":index.is_some()})
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "ok": checks.iter().all(|check| check["matched"] == true),
        "checks": checks
    })
}

fn best_ordered_check_matches(
    calls: &[TranscriptCall<'_>],
    required: &[RequiredToolCall],
) -> Vec<bool> {
    let mut best = vec![false; required.len()];
    let mut best_count = 0;
    for start in 0..calls.len() {
        let matches = required
            .iter()
            .enumerate()
            .map(|(index, required)| {
                calls
                    .get(start + index)
                    .is_some_and(|call| call_matches(*call, required))
            })
            .collect::<Vec<_>>();
        let count = matches.iter().filter(|matched| **matched).count();
        if count > best_count {
            best = matches;
            best_count = count;
        }
    }
    best
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
    fn schema_v2_completed_gate_recovers_malformed_candidate_without_reexecuting_tools() {
        let todo = required_call_todo();
        let gate = evaluate_gate(2, &todo, &tool_messages(false));

        let candidate = resolve_candidate(2, &todo, Some("not-json"), &gate, true).unwrap();

        assert_eq!(candidate.status, CandidateStatus::Candidate);
        assert!(candidate
            .summary
            .contains("every declared required tool call succeeded"));
        assert_eq!(candidate.result.as_deref(), Some("x"));
    }

    #[test]
    fn candidate_recovery_requires_schema_v2_nonempty_completed_gate() {
        let todo = required_call_todo();
        let passed = evaluate_gate(2, &todo, &tool_messages(false));
        let failed = evaluate_gate(2, &todo, &[]);
        let mut no_calls = todo.clone();
        no_calls.acceptance.required_tool_calls.clear();

        assert!(resolve_candidate(1, &todo, Some("not-json"), &passed, true).is_err());
        assert!(resolve_candidate(2, &todo, Some("not-json"), &failed, true).is_err());
        assert!(resolve_candidate(2, &no_calls, None, &passed, true).is_err());
    }

    #[test]
    fn blocked_candidate_is_preserved_when_runtime_did_not_finish_the_gate() {
        let todo = required_call_todo();
        let gate = evaluate_gate(2, &todo, &tool_messages(false));

        let candidate = resolve_candidate(2, &todo, Some(BLOCKED_CANDIDATE), &gate, false).unwrap();

        assert_eq!(candidate.status, CandidateStatus::Blocked);
    }

    #[test]
    fn runtime_completed_gate_overrides_cancelled_blocked_tail() {
        let todo = required_call_todo();
        let gate = evaluate_gate(2, &todo, &tool_messages(false));

        let candidate = resolve_candidate(2, &todo, Some(BLOCKED_CANDIDATE), &gate, true).unwrap();

        assert_eq!(candidate.status, CandidateStatus::Candidate);
    }

    #[test]
    fn completion_gate_stops_only_after_all_required_calls_succeed_in_order() {
        let mut todo = required_call_todo();
        todo.acceptance.required_tool_calls.push(RequiredToolCall {
            name: "mcp__fk__observe".into(),
            arguments_contains: serde_json::json!({"command":"observe"}),
            result_ok: true,
        });
        let mut gate = CompletionGate::new(2, &todo);
        assert!(!gate.observe(&SessionEvent::ToolStart {
            id: "tap".into(),
            name: "mcp__fk__tap".into(),
            input: serde_json::json!({"label":"A","x":1}),
        }));
        assert!(!gate.observe(&SessionEvent::ToolEnd {
            id: "tap".into(),
            name: "mcp__fk__tap".into(),
            output: "ok".into(),
            is_error: false,
            images: vec![],
        }));
        assert!(!gate.observe(&SessionEvent::ToolStart {
            id: "observe".into(),
            name: "mcp__fk__observe".into(),
            input: serde_json::json!({"command":"observe"}),
        }));
        assert!(gate.observe(&SessionEvent::ToolEnd {
            id: "observe".into(),
            name: "mcp__fk__observe".into(),
            output: "ok".into(),
            is_error: false,
            images: vec![],
        }));
    }

    #[test]
    fn completion_gate_does_not_stop_on_failed_or_out_of_order_calls() {
        let mut todo = required_call_todo();
        todo.acceptance.required_tool_calls.push(RequiredToolCall {
            name: "mcp__fk__observe".into(),
            arguments_contains: serde_json::json!({"command":"observe"}),
            result_ok: true,
        });
        let mut gate = CompletionGate::new(2, &todo);
        for event in [
            SessionEvent::ToolStart {
                id: "observe".into(),
                name: "mcp__fk__observe".into(),
                input: serde_json::json!({"command":"observe"}),
            },
            SessionEvent::ToolEnd {
                id: "observe".into(),
                name: "mcp__fk__observe".into(),
                output: "ok".into(),
                is_error: false,
                images: vec![],
            },
            SessionEvent::ToolStart {
                id: "tap".into(),
                name: "mcp__fk__tap".into(),
                input: serde_json::json!({"label":"A","x":1}),
            },
            SessionEvent::ToolEnd {
                id: "tap".into(),
                name: "mcp__fk__tap".into(),
                output: "failed".into(),
                is_error: true,
                images: vec![],
            },
        ] {
            assert!(!gate.observe(&event));
        }
    }

    #[test]
    fn completion_gate_rejects_interleaved_undeclared_calls() {
        let mut todo = required_call_todo();
        todo.acceptance.required_tool_calls.push(RequiredToolCall {
            name: "mcp__fk__observe".into(),
            arguments_contains: serde_json::json!({"command":"observe"}),
            result_ok: true,
        });
        let calls = vec![
            ObservedCall {
                id: "tap".into(),
                name: "mcp__fk__tap".into(),
                input: serde_json::json!({"label":"A","x":1}),
                ok: Some(true),
            },
            ObservedCall {
                id: "extra".into(),
                name: "mcp__fk__back".into(),
                input: serde_json::json!({"command":"back"}),
                ok: Some(true),
            },
            ObservedCall {
                id: "observe".into(),
                name: "mcp__fk__observe".into(),
                input: serde_json::json!({"command":"observe"}),
                ok: Some(true),
            },
        ];

        assert!(!ordered_required_calls_match(
            &calls,
            &todo.acceptance.required_tool_calls
        ));
    }

    #[test]
    fn schema_v2_transcript_gate_rejects_interleaved_undeclared_calls() {
        let mut todo = required_call_todo();
        todo.acceptance.required_tool_calls.push(RequiredToolCall {
            name: "mcp__fk__observe".into(),
            arguments_contains: serde_json::json!({"command":"observe"}),
            result_ok: true,
        });
        let mut messages = tool_messages(false);
        let mut extra_start = Message::assistant("extra-start");
        extra_start.blocks = vec![ContentBlock::ToolUse {
            id: "extra".into(),
            name: "mcp__fk__back".into(),
            input: serde_json::json!({"command":"back"}),
        }];
        let mut extra_end = Message::user("extra-end", "");
        extra_end.role = Role::Tool;
        extra_end.blocks = vec![ContentBlock::ToolResult {
            tool_use_id: "extra".into(),
            content: "ok".into(),
            is_error: false,
            images: vec![],
        }];
        let mut observe_start = Message::assistant("observe-start");
        observe_start.blocks = vec![ContentBlock::ToolUse {
            id: "observe".into(),
            name: "mcp__fk__observe".into(),
            input: serde_json::json!({"command":"observe"}),
        }];
        let mut observe_end = Message::user("observe-end", "");
        observe_end.role = Role::Tool;
        observe_end.blocks = vec![ContentBlock::ToolResult {
            tool_use_id: "observe".into(),
            content: "ok".into(),
            is_error: false,
            images: vec![],
        }];
        messages.extend([extra_start, extra_end, observe_start, observe_end]);

        assert_eq!(evaluate_gate(2, &todo, &messages)["ok"], false);
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
            allowed_tools: vec![],
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
        assert_eq!(evaluate_gate(1, &todo, &tool_messages(false))["ok"], true);
        assert_eq!(allowed_tool_names(&todo), vec!["mcp__fk__tap"]);
    }

    fn required_call_todo() -> TodoSpec {
        TodoSpec {
            id: "x".into(),
            title: "x".into(),
            requirement_background: "x".into(),
            instructions: "x".into(),
            depends_on: vec![],
            agent: "act".into(),
            max_attempts: 1,
            allowed_tools: vec![],
            acceptance: AcceptanceSpec {
                criteria: "x".into(),
                required_tool_calls: vec![RequiredToolCall {
                    name: "mcp__fk__tap".into(),
                    arguments_contains: serde_json::json!({"label":"A"}),
                    result_ok: true,
                }],
            },
            metadata: serde_json::Value::Null,
        }
    }

    fn assistant_text(id: &str, text: &str) -> Message {
        let mut message = Message::assistant(id);
        message.blocks = vec![opencoder_core::ContentBlock::text(text)];
        message
    }

    #[test]
    fn latest_new_assistant_ignores_pre_watermark_history() {
        let messages = vec![
            Message::user("u1", "previous attempt"),
            assistant_text("a1", BLOCKED_CANDIDATE),
        ];

        // Watermark at the transcript tail: nothing new was produced, so the
        // previous attempt's assistant message must NOT be recycled.
        assert_eq!(latest_new_assistant(&messages, messages.len()), None);
    }

    #[test]
    fn latest_new_assistant_prefers_post_watermark_message() {
        let stale = assistant_text("a1", BLOCKED_CANDIDATE);
        let fresh = assistant_text("a2", r#"{"status":"candidate"}"#);
        let mut messages = vec![Message::user("u1", "previous attempt"), stale.clone()];
        let watermark = messages.len();
        messages.push(Message::user("u2", "this run"));
        messages.push(fresh.clone());

        assert_eq!(
            latest_new_assistant(&messages, watermark),
            Some(r#"{"status":"candidate"}"#.to_string())
        );
        // Degenerate watermark keeps the historical "latest assistant" rule.
        assert_eq!(
            latest_new_assistant(&messages, 0),
            Some(r#"{"status":"candidate"}"#.to_string())
        );
        assert_eq!(stale.role, Role::Assistant);
        assert_eq!(fresh.role, Role::Assistant);
    }

    #[test]
    fn latest_new_assistant_skips_new_user_only_tail() {
        let messages = vec![
            assistant_text("a1", BLOCKED_CANDIDATE),
            Message::user("u2", "this run added no assistant message"),
        ];

        assert_eq!(latest_new_assistant(&messages, 1), None);
    }

    fn tool_messages(is_error: bool) -> Vec<Message> {
        let mut start = Message::assistant("tool-start");
        start.blocks = vec![ContentBlock::ToolUse {
            id: "1".into(),
            name: "mcp__fk__tap".into(),
            input: serde_json::json!({"label":"A","x":1}),
        }];
        let mut end = Message::user("tool-end", "");
        end.role = Role::Tool;
        end.blocks = vec![ContentBlock::ToolResult {
            tool_use_id: "1".into(),
            content: if is_error { "boom" } else { "ok" }.into(),
            is_error,
            images: vec![],
        }];
        vec![start, end]
    }

    #[test]
    fn gate_rejects_when_required_call_missing() {
        let todo = required_call_todo();
        let mut messages = tool_messages(false);
        messages.pop();

        let gate = evaluate_gate(1, &todo, &messages);

        assert_eq!(gate["ok"], false);
        assert_eq!(gate["checks"][0]["matched"], false);
    }

    #[test]
    fn gate_rejects_errored_tool_end() {
        let todo = required_call_todo();
        let gate = evaluate_gate(1, &todo, &tool_messages(true));

        assert_eq!(gate["ok"], false);
        assert_eq!(gate["checks"][0]["matched"], false);
    }

    #[test]
    fn ordered_gate_does_not_reuse_one_call() {
        let mut todo = required_call_todo();
        todo.acceptance
            .required_tool_calls
            .push(todo.acceptance.required_tool_calls[0].clone());
        let gate = evaluate_gate(2, &todo, &tool_messages(false));
        assert_eq!(gate["checks"][0]["matched"], true);
        assert_eq!(gate["checks"][1]["matched"], false);
    }
}
