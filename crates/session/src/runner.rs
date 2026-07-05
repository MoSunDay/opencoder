use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use opencode_core::{message::now_ms, resolve_agent, ContentBlock, Message, MessageUsage, Role, ToolArc, ToolContext, ToolOutput};
use opencode_llm::tool_call::CompletedToolCall;
use opencode_llm::{lower_messages, ChatRequest, ChatStream, LlmEvent, Usage};
use serde_json::Value;

use crate::compaction;
use crate::prompt::{build_system, plan_to_act_note};
use crate::tools::{registry as build_registry, schema_for};
use crate::SessionState;

#[derive(Debug, Clone)]
pub enum SessionEvent {
    TextDelta(String),
    ReasoningDelta(String),
    ToolStart { id: String, name: String, input: Value },
    ToolEnd { id: String, name: String, output: String, is_error: bool },
    AgentSwitch(String),
    Compaction(String),
    Status(String),
    /// A subagent (task tool) started. `kind` is the subagent_type, `prompt` its task.
    SubagentStart { id: String, kind: String, prompt: String },
    /// A subagent finished. `depth` is nesting level (1 = direct child).
    SubagentEnd { id: String, ok: bool, summary: String },
    Done,
    Error(String),
}

const MAX_OUTPUT: usize = 20_000;
const DOOM_THRESHOLD: usize = 3;

/// True if the session's cancellation token has been tripped (hard-abort).
fn cancelled(session: &SessionState) -> bool {
    session.cancel.as_ref().map(|c| c.is_cancelled()).unwrap_or(false)
}

/// Resolves when the session is cancelled. If no token is attached, this never
/// resolves (pending forever), so the `select!` cancel arm stays dormant.
async fn await_cancel(session: &SessionState) {
    match session.cancel.as_ref() {
        Some(c) => c.cancelled().await,
        None => std::future::pending::<()>().await,
    }
}

pub async fn run(
    session: &mut SessionState,
    user_text: String,
    on_event: impl FnMut(SessionEvent) + Send,
) -> Result<()> {
    let registry = build_registry();
    run_with_registry(session, user_text, &registry, on_event).await
}

pub async fn run_with_registry(
    session: &mut SessionState,
    user_text: String,
    registry: &HashMap<String, ToolArc>,
    on_event: impl FnMut(SessionEvent) + Send,
) -> Result<()> {
    let mut on_event = on_event;
    // An empty prompt means "drain mode": don't push a synthetic user message —
    // the web drain relies on admitted steers/queues being claimed at turn
    // boundaries to supply the actual user input.
    if !user_text.trim().is_empty() {
        let user = Message::user(new_id(), user_text);
        session.record(user).await;
    }
    run_loop(session, registry, &mut on_event).await
}

async fn run_loop(
    session: &mut SessionState,
    registry: &HashMap<String, ToolArc>,
    on_event: &mut (impl FnMut(SessionEvent) + Send),
) -> Result<()> {
    let max_steps = session.agent.max_steps.max(1);
    let mut doom: VecDeque<String> = VecDeque::new();

    loop {
        // Interrupt check: if a cancellation was requested (web POST /interrupt),
        // stop cleanly at this turn boundary.
        if let Some(c) = &session.cancel {
            if c.is_cancelled() {
                on_event(SessionEvent::Status("interrupted".into()));
                break;
            }
        }
        // Safe Provider-Turn Boundary: promote any steers admitted since the
        // last turn. A steer is absorbed into history HERE and resets the
        // agent's step allowance to 1 (fresh continuation budget).
        let steer_prompts = claim_steers(session).await;
        if !steer_prompts.is_empty() {
            for p in &steer_prompts {
                let mut m = Message::user(new_id(), p.clone());
                m.synthetic = true;
                session.record(m).await;
            }
            session.step = 0; // incremented to 1 below → reset allowance
            on_event(SessionEvent::Status(format!("steer promoted ({} new input(s))", steer_prompts.len())));
        }

        session.step += 1;
        if session.step > max_steps {
            on_event(SessionEvent::Status(format!("reached max steps ({max_steps}), stopping")));
            break;
        }
        if compaction::should_compact(session) {
            match compaction::compact(session, registry).await {
                Ok(summary) => on_event(SessionEvent::Compaction(summary)),
                Err(e) => on_event(SessionEvent::Error(format!("compaction failed: {e:#}"))),
            }
        }

        let turn = match run_one_llm_call(session, registry, on_event).await {
            Ok(t) => t,
            Err(e) => {
                on_event(SessionEvent::Error(format!("{e:#}")));
                return Err(e);
            }
        };
        let (text, tool_calls, usage) = turn;
        if let Some(u) = &usage {
            session.last_usage = u.clone();
        }

        let mut blocks: Vec<ContentBlock> = Vec::new();
        if !text.is_empty() {
            blocks.push(ContentBlock::Text { text });
        }
        for tc in &tool_calls {
            blocks.push(ContentBlock::ToolUse {
                id: tc.id.clone(),
                name: tc.name.clone(),
                input: tc.input.clone(),
            });
        }
        let mut assistant = Message::assistant(new_id());
        assistant.model = Some(session.model.clone());
        assistant.agent = Some(session.agent.name.clone());
        assistant.blocks = blocks;
        assistant.usage = usage.as_ref().map(core_usage).unwrap_or_default();
        assistant.created_at = now_ms();
        session.record(assistant).await;

        if tool_calls.is_empty() {
            // Idle boundary: consume exactly ONE queued follow-up, if any. A
            // queued input does NOT reset the step counter and only fires when
            // the session would otherwise go idle.
            if let Some(q) = claim_one_queued(session).await {
                let mut m = Message::user(new_id(), q);
                m.synthetic = true;
                session.record(m).await;
                on_event(SessionEvent::Status("queued follow-up promoted".into()));
                continue;
            }
            on_event(SessionEvent::Done);
            break;
        }

        let mut tool_msg = Message {
            id: new_id(),
            role: Role::Tool,
            blocks: Vec::new(),
            model: None,
            agent: None,
            usage: MessageUsage::default(),
            created_at: now_ms(),
            synthetic: false,
        };
        let mut switched_to_act = false;
        for tc in tool_calls {
            let sig = format!("{}:{}", tc.name, tc.input);
            doom.push_back(sig.clone());
            if doom.len() > DOOM_THRESHOLD {
                doom.pop_front();
            }
            if doom.len() == DOOM_THRESHOLD && doom.iter().all(|s| s == &sig) {
                on_event(SessionEvent::Error("doom-loop: same tool repeated 3x, stopping".into()));
                return Ok(());
            }
            on_event(SessionEvent::ToolStart {
                id: tc.id.clone(),
                name: tc.name.clone(),
                input: tc.input.clone(),
            });
            let out = execute_call(&tc, session, registry, &mut *on_event).await;
            on_event(SessionEvent::ToolEnd {
                id: tc.id.clone(),
                name: tc.name.clone(),
                output: out.content.clone(),
                is_error: out.is_error,
            });
            if cancelled(session) {
                break;
            }
            if tc.name == "plan_exit" {
                switched_to_act = true;
            }
            tool_msg.blocks.push(ContentBlock::ToolResult {
                tool_use_id: tc.id.clone(),
                content: out.content,
                is_error: out.is_error,
            });
        }
        session.record(tool_msg).await;

        if switched_to_act {
            if let Some(act) = resolve_agent("act") {
                session.agent = act;
                on_event(SessionEvent::AgentSwitch("act".into()));
            }
            let mut note = Message::user(new_id(), plan_to_act_note());
            note.synthetic = true;
            session.record(note).await;
        }
    }
    Ok(())
}

async fn run_one_llm_call(
    session: &SessionState,
    registry: &HashMap<String, ToolArc>,
    on_event: &mut (impl FnMut(SessionEvent) + Send),
) -> Result<(String, Vec<CompletedToolCall>, Option<Usage>)> {
    let system = build_system(&session.agent, &session.working_dir, session.skill_prompt.as_deref());
    let mut to_send = vec![system];
    to_send.extend(session.messages.iter().cloned());
    let openai_msgs = lower_messages(&to_send);

    let allowed: HashMap<String, ToolArc> = registry
        .iter()
        .filter(|(name, _)| session.agent.tools.allows(name))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let tool_schemas = schema_for(&allowed);

    let req = ChatRequest {
        model: session.model.clone(),
        messages: openai_msgs,
        tools: tool_schemas,
        tool_choice: if allowed.is_empty() { None } else { Some("auto".into()) },
        temperature: None,
        max_tokens: session.config.max_tokens,
    };
    let mut rx = session.client.chat_stream(req)?;
    let mut completed: Option<(String, Vec<CompletedToolCall>, Option<Usage>)> = None;
    let mut cancel_fut = std::pin::pin!(await_cancel(session));
    loop {
        tokio::select! {
            biased;
            _ = &mut cancel_fut => {
                on_event(SessionEvent::Status("interrupted".into()));
                return Ok((String::new(), Vec::new(), None));
            }
            ev = rx.recv() => {
                let ev = match ev { Some(ev) => ev, None => break };
                match ev {
                    LlmEvent::TextDelta(t) => on_event(SessionEvent::TextDelta(t)),
                    LlmEvent::ReasoningDelta(r) => on_event(SessionEvent::ReasoningDelta(r)),
                    LlmEvent::ToolCallStart { .. } | LlmEvent::ToolCallDelta { .. } => {}
                    LlmEvent::Completed { text, tool_calls, usage } => {
                        completed = Some((text, tool_calls, usage));
                    }
                    LlmEvent::Error(e) => return Err(anyhow!(e)),
                }
            }
        }
    }
    completed.ok_or_else(|| anyhow!("stream ended without completion"))
}

async fn execute_call(
    tc: &CompletedToolCall,
    session: &SessionState,
    registry: &HashMap<String, ToolArc>,
    on_event: &mut (dyn FnMut(SessionEvent) + Send),
) -> ToolOutput {
    if tc.name == "task" {
        return run_subagent(tc.input.clone(), tc.id.clone(), session, registry, on_event).await;
    }
    let ctx = ToolContext {
        session_id: session.id.clone(),
        message_id: tc.id.clone(),
        agent: session.agent.name.clone(),
        working_dir: session.working_dir.clone(),
        max_output: MAX_OUTPUT,
    };
    match registry.get(&tc.name) {
        Some(tool) => {
            let mut cancel_fut = std::pin::pin!(await_cancel(session));
            let exec = tool.execute(tc.input.clone(), &ctx);
            tokio::select! {
                biased;
                _ = &mut cancel_fut => ToolOutput::err("interrupted"),
                o = exec => o.unwrap_or_else(|e| ToolOutput::err(format!("{e:#}"))),
            }
        }
        None => ToolOutput::err(format!("unknown tool: {}", tc.name)),
    }
}

async fn run_subagent(
    input: Value,
    call_id: String,
    parent: &SessionState,
    registry: &HashMap<String, ToolArc>,
    on_event: &mut (dyn FnMut(SessionEvent) + Send),
) -> ToolOutput {
    let prompt = input.get("prompt").and_then(|v| v.as_str()).unwrap_or("").to_string();
    if prompt.is_empty() {
        return ToolOutput::err("task requires a prompt");
    }
    let kind = input.get("subagent_type").and_then(|v| v.as_str()).unwrap_or("subagent").to_string();
    let agent = match resolve_agent(&kind).or_else(|| resolve_agent("subagent")) {
        Some(a) => a,
        None => return ToolOutput::err("no subagent available"),
    };
    let preview: String = prompt.chars().take(80).collect();
    on_event(SessionEvent::SubagentStart { id: call_id.clone(), kind: kind.clone(), prompt: preview });

    let mut child = SessionState::new(
        format!("sub-{}", new_id()),
        agent,
        parent.config.clone(),
        parent.client.clone(),
        parent.working_dir.clone(),
    );
    // Propagate the parent's cancellation token so a double-Esc also stops a
    // running subagent at its next turn boundary.
    child.cancel = parent.cancel.clone();
    // Forward child events to the parent sink so the UI can show subagent work.
    // Tool calls are surfaced verbatim; text/reasoning are folded into the
    // SubagentEnd summary to avoid flooding the parent transcript.
    let mut child_chars = String::new();
    let mut child_tools: u32 = 0;
    let summary_chars = &mut child_chars;
    let tool_count = &mut child_tools;
    let res = Box::pin(run_with_registry(
        &mut child,
        prompt.clone(),
        registry,
        |cev| {
            match cev {
                SessionEvent::ToolStart { id, name, input } => {
                    *tool_count += 1;
                    on_event(SessionEvent::ToolStart { id, name, input });
                }
                SessionEvent::ToolEnd { id, name, output, is_error } => {
                    on_event(SessionEvent::ToolEnd { id, name, output, is_error });
                }
                SessionEvent::TextDelta(t) => {
                    if summary_chars.len() < 240 {
                        summary_chars.push_str(&t);
                    }
                }
                SessionEvent::Error(e) => {
                    on_event(SessionEvent::Status(format!("subagent error: {e}")));
                }
                _ => {}
            }
        },
    ))
    .await;

    let ok = res.is_ok();
    let text = child
        .messages
        .iter()
        .rev()
        .find(|m| m.role == Role::Assistant)
        .map(|m| m.text())
        .unwrap_or_default();
    let summary_preview: String = if child_chars.is_empty() {
        text.chars().take(120).collect()
    } else {
        child_chars.chars().take(120).collect()
    };
    on_event(SessionEvent::SubagentEnd {
        id: call_id.clone(),
        ok,
        summary: format!("({} tool calls) {}", child_tools, summary_preview),
    });
    if ok {
        ToolOutput::ok(text)
    } else {
        ToolOutput::err("subagent failed")
    }
}

fn core_usage(u: &Usage) -> MessageUsage {
    MessageUsage {
        input_tokens: u.input_tokens,
        output_tokens: u.output_tokens,
        total_tokens: u.total_tokens,
    }
}

/// Claim all pending steer inputs at a turn boundary: read them, mark promoted,
/// return their prompts to be appended to history. No-op when no store is
/// attached or none pending. Idempotent (promote only touches NULL promoted_seq).
async fn claim_steers(session: &mut SessionState) -> Vec<String> {
    let store = match session.store.clone() {
        Some(s) => s,
        None => return Vec::new(),
    };
    let pending = match store.pending_inputs(&session.id, opencode_store::Delivery::Steer).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "claim_steers: pending_inputs failed");
            return Vec::new();
        }
    };
    if pending.is_empty() {
        return Vec::new();
    }
    if let Err(e) = store
        .promote_inputs(&session.id, i64::MAX, opencode_store::Delivery::Steer)
        .await
    {
        tracing::warn!(error = %e, "claim_steers: promote_inputs failed");
        return Vec::new();
    }
    pending.into_iter().map(|i| i.prompt).collect()
}

/// Claim exactly one queued input at idle. Returns its prompt, or None.
async fn claim_one_queued(session: &mut SessionState) -> Option<String> {
    let store = session.store.clone()?;
    match store.claim_next_queue(&session.id).await {
        Ok(Some(input)) => Some(input.prompt),
        Ok(None) => None,
        Err(e) => {
            tracing::warn!(error = %e, "claim_one_queued failed");
            None
        }
    }
}

pub fn new_id() -> String {
    ulid::Ulid::new().to_string()
}

pub async fn run_once(
    agent_name: &str,
    config: opencode_core::Config,
    client: Arc<dyn ChatStream>,
    working_dir: std::path::PathBuf,
    prompt: String,
    on_event: impl FnMut(SessionEvent) + Send,
) -> Result<SessionState> {
    let agent = resolve_agent(agent_name)
        .or_else(|| resolve_agent("act"))
        .ok_or_else(|| anyhow!("no default agent"))?;
    let mut session = SessionState::new(new_id(), agent, config, client, working_dir);
    run(&mut session, prompt, on_event).await?;
    Ok(session)
}
