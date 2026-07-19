use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use futures::stream::{FuturesUnordered, StreamExt};
use opencoder_core::{
    message::now_ms, resolve_agent, AgentKind, ContentBlock, Message, MessageUsage, Role, ToolArc,
    ToolContext, ToolOutput,
};
use opencoder_llm::tool_call::CompletedToolCall;
use opencoder_llm::{lower_messages, ChatRequest, ChatStream, LlmEvent, Usage};
use opencoder_store::{EventKind, SessionEventRecord, SubagentStatus, SubagentTaskRecord};
use serde_json::Value;

use crate::compaction;
use crate::prompt::build_system;
use crate::tools::{registry as build_registry, schema_for};
use crate::SessionState;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SessionEvent {
    TextDelta(String),
    ReasoningDelta(String),
    ToolStart {
        id: String,
        name: String,
        input: Value,
    },
    ToolEnd {
        id: String,
        name: String,
        output: String,
        is_error: bool,
    },
    AgentSwitch(String),
    Compaction(String),
    Status(String),
    /// A subagent (task tool) started. `child_session_id` is the child's
    /// session for loading its transcript from the store.
    SubagentStart {
        id: String,
        kind: String,
        prompt: String,
        child_session_id: String,
    },
    /// A subagent finished.
    SubagentEnd {
        id: String,
        ok: bool,
        summary: String,
    },
    /// A child event from a running subagent, tagged with the tool-call id so
    /// the TUI can route it into the subagent's foldable block.
    SubagentChild {
        id: String,
        ev: Box<SessionEvent>,
    },
    /// Emitted after compaction rewrites the transcript. Carries the new
    /// message list so display surfaces can rebuild their view.
    TranscriptReset(Vec<opencoder_core::Message>),
    /// Emitted after plan→act handoff. Carries the plan text (markdown) for the
    /// display layer to render as a read-only card. Paired with a preceding
    /// TranscriptReset that rebuilds the clean view.
    PlanHandoff(String),
    /// A queued follow-up was consumed (drained) at an idle boundary. Carries
    /// the consumed input's row seq so the TUI can drop it from its pending
    /// mirror instead of leaving a stale `[queued]` row until `Done`.
    QueueConsumed {
        seq: i64,
    },
    /// A steered input was consumed (promoted) at a turn boundary. Carries
    /// the consumed input's row seq so the TUI can drop it from its pending
    /// mirror instead of leaving a stale `steer` row until `Done`.
    SteerConsumed {
        seq: i64,
    },
    Done,
    Error(String),
}

impl SessionEvent {
    /// The granular SSE event-name string for this variant.
    /// Single source of truth shared by the web layer (live broadcast +
    /// replay) and the TUI (persistence), so a session driven by either
    /// surface replays identically.
    pub fn sse_kind(&self) -> &'static str {
        match self {
            SessionEvent::TextDelta(_) => "text_delta",
            SessionEvent::ReasoningDelta(_) => "reasoning_delta",
            SessionEvent::ToolStart { .. } => "tool_start",
            SessionEvent::ToolEnd { .. } => "tool_end",
            SessionEvent::AgentSwitch(_) => "agent_switched",
            SessionEvent::Compaction(_) => "compaction",
            SessionEvent::Status(_) => "status",
            SessionEvent::Done => "done",
            SessionEvent::Error(_) => "error",
            SessionEvent::SubagentStart { .. } => "subagent_start",
            SessionEvent::SubagentEnd { .. } => "subagent_end",
            SessionEvent::SubagentChild { .. } => "subagent_child",
            SessionEvent::PlanHandoff(_) => "plan_handoff",
            SessionEvent::TranscriptReset(_) => "transcript_reset",
            SessionEvent::QueueConsumed { .. } => "queue_consumed",
            SessionEvent::SteerConsumed { .. } => "steer_consumed",
        }
    }

    /// The structured JSON payload for this variant, matching the SSE wire
    /// format. Both web and TUI use this for persistence so the replayed
    /// payload shape is identical to the live broadcast.
    pub fn sse_data(&self) -> serde_json::Value {
        match self {
            SessionEvent::TextDelta(t) => serde_json::json!({ "text": t }),
            SessionEvent::ReasoningDelta(r) => serde_json::json!({ "text": r }),
            SessionEvent::ToolStart { id, name, input } => {
                serde_json::json!({ "id": id, "name": name, "input": input })
            }
            SessionEvent::ToolEnd { id, name, output, is_error } => {
                serde_json::json!({ "id": id, "name": name, "output": output, "is_error": is_error })
            }
            SessionEvent::AgentSwitch(a) => serde_json::json!({ "agent": a }),
            SessionEvent::Compaction(s) => serde_json::json!({ "summary": s }),
            SessionEvent::Status(s) => serde_json::json!({ "status": s }),
            SessionEvent::Done => serde_json::json!({}),
            SessionEvent::Error(e) => serde_json::json!({ "error": e }),
            SessionEvent::SubagentStart { id, kind, prompt, child_session_id } => {
                serde_json::json!({ "id": id, "kind": kind, "prompt": prompt, "child_session_id": child_session_id })
            }
            SessionEvent::SubagentEnd { id, ok, summary } => {
                serde_json::json!({ "id": id, "ok": ok, "summary": summary })
            }
            SessionEvent::SubagentChild { id, ev } => {
                serde_json::json!({ "id": id, "event": ev })
            }
            SessionEvent::PlanHandoff(plan) => serde_json::json!({ "plan": plan }),
            SessionEvent::TranscriptReset(_) => serde_json::json!({}),
            SessionEvent::QueueConsumed { seq } => serde_json::json!({ "seq": seq }),
            SessionEvent::SteerConsumed { seq } => serde_json::json!({ "seq": seq }),
        }
    }

    /// Reconstruct a `SessionEvent` from an SSE event-name (`sse_kind`) and its
    /// payload (`sse_data`). This is the inverse of `sse_kind()` + `sse_data()`,
    /// letting a remote client (`opencode client`) rebuild the structured event
    /// stream from a server's `/events` SSE wire format.
    ///
    /// Returns `None` for an unrecognized `kind`. `TranscriptReset` carries no
    /// messages on the wire (its payload is `{}`), so it is returned as an empty
    /// marker — callers that need the rebuilt transcript must re-fetch
    /// `/messages`. `SubagentChild` deserializes its nested `event` as the enum
    /// (not the SSE form), matching how `sse_data` serializes it.
    pub fn from_sse(kind: &str, data: serde_json::Value) -> Option<Self> {
        Some(match kind {
            "text_delta" => SessionEvent::TextDelta(data.get("text")?.as_str()?.to_string()),
            "reasoning_delta" => {
                SessionEvent::ReasoningDelta(data.get("text")?.as_str()?.to_string())
            }
            "tool_start" => SessionEvent::ToolStart {
                id: data.get("id")?.as_str()?.to_string(),
                name: data.get("name")?.as_str()?.to_string(),
                input: data.get("input")?.clone(),
            },
            "tool_end" => SessionEvent::ToolEnd {
                id: data.get("id")?.as_str()?.to_string(),
                name: data.get("name")?.as_str()?.to_string(),
                output: data.get("output")?.as_str()?.to_string(),
                is_error: data.get("is_error")?.as_bool().unwrap_or(false),
            },
            "agent_switched" => {
                SessionEvent::AgentSwitch(data.get("agent")?.as_str()?.to_string())
            }
            "compaction" => {
                SessionEvent::Compaction(data.get("summary")?.as_str()?.to_string())
            }
            "status" => SessionEvent::Status(data.get("status")?.as_str()?.to_string()),
            "subagent_start" => SessionEvent::SubagentStart {
                id: data.get("id")?.as_str()?.to_string(),
                kind: data.get("kind")?.as_str()?.to_string(),
                prompt: data.get("prompt")?.as_str()?.to_string(),
                child_session_id: data.get("child_session_id")?.as_str()?.to_string(),
            },
            "subagent_end" => SessionEvent::SubagentEnd {
                id: data.get("id")?.as_str()?.to_string(),
                ok: data.get("ok")?.as_bool().unwrap_or(false),
                summary: data.get("summary")?.as_str()?.to_string(),
            },
            "subagent_child" => {
                let ev: SessionEvent =
                    serde_json::from_value(data.get("event")?.clone()).ok()?;
                SessionEvent::SubagentChild {
                    id: data.get("id")?.as_str()?.to_string(),
                    ev: Box::new(ev),
                }
            }
            "plan_handoff" => {
                SessionEvent::PlanHandoff(data.get("plan")?.as_str()?.to_string())
            }
            "transcript_reset" => {
                // Wire payload is `{}`; the rebuilt message list is intentionally
                // empty (see method doc). Callers re-fetch /messages if needed.
                SessionEvent::TranscriptReset(Vec::new())
            }
            "queue_consumed" => SessionEvent::QueueConsumed {
                seq: data.get("seq")?.as_i64().unwrap_or(0),
            },
            "steer_consumed" => SessionEvent::SteerConsumed {
                seq: data.get("seq")?.as_i64().unwrap_or(0),
            },
            "done" => SessionEvent::Done,
            "error" => SessionEvent::Error(data.get("error")?.as_str()?.to_string()),
            _ => return None,
        })
    }

    /// Coarse [`EventKind`] for backward-compatible DB `type` column.
    pub fn coarse_kind(&self) -> EventKind {
        match self {
            SessionEvent::TextDelta(_) => EventKind::TextDelta,
            SessionEvent::ReasoningDelta(_) => EventKind::TextDelta,
            SessionEvent::ToolStart { .. } => EventKind::ToolStart,
            SessionEvent::ToolEnd { .. } => EventKind::ToolEnd,
            SessionEvent::AgentSwitch(_) => EventKind::AgentSwitched,
            SessionEvent::Compaction(_) => EventKind::Compaction,
            SessionEvent::Status(_) => EventKind::Step,
            SessionEvent::Done => EventKind::Done,
            SessionEvent::Error(_) => EventKind::Error,
            SessionEvent::SubagentStart { .. }
            | SessionEvent::SubagentEnd { .. }
            | SessionEvent::SubagentChild { .. }
            | SessionEvent::PlanHandoff(_)
            | SessionEvent::QueueConsumed { .. }
            | SessionEvent::SteerConsumed { .. } => EventKind::Step,
            SessionEvent::TranscriptReset(_) => EventKind::Compaction,
        }
    }
}

const MAX_OUTPUT: usize = 4096;
const DOOM_THRESHOLD: usize = 3;

/// Shared event sink for concurrent tool dispatch. Wraps the borrowed `FnMut`
/// closure in a `Mutex` so multiple in-flight tool/subagent futures can emit
/// events safely (emissions serialize; each is a fast push). The lifetime is
/// bound to the caller's closure — no `'static` requirement, so test closures
/// that borrow local state keep working unmodified.
type Sink<'a> = Arc<Mutex<&'a mut (dyn FnMut(SessionEvent) + Send)>>;

/// Emit an event through the shared sink. Best-effort: a poisoned mutex (only
/// possible on panic inside a closure) drops the event rather than propagating.
fn emit(sink: &Sink<'_>, ev: SessionEvent) {
    if let Ok(mut g) = sink.lock() {
        // g: MutexGuard<&mut (dyn FnMut + Send)>; deref to the inner closure
        // reference and call it.
        (**g)(ev);
    }
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
    // A non-empty prompt records a real user message. An empty prompt means
    // "drain mode": the web drain relies on admitted steers/queues being
    // claimed at turn boundaries to supply the actual user input, and the web
    // has no skill support (`skill_prompt` is `None`). But for skill-only
    // submits (empty prompt with an active skill), inject a synthetic trigger
    // message so the model records a user turn and acts on the skill body in
    // the system prompt instead of treating it passively.
    if !user_text.trim().is_empty() {
        let user = Message::user(new_id(), user_text);
        session.record(user).await;
    } else if session.skill_prompt_cloned().is_some() {
        let mut msg = Message::user(
            new_id(),
            "The active skill is now in effect. Begin executing it now.",
        );
        msg.synthetic = true;
        session.record(msg).await;
    }
    run_loop(session, registry, &mut on_event).await
}

async fn run_loop(
    session: &mut SessionState,
    registry: &HashMap<String, ToolArc>,
    on_event: &mut (dyn FnMut(SessionEvent) + Send),
) -> Result<()> {
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
        // last turn. A steer is absorbed into history HERE.
        let steer_prompts = claim_steers(session).await;
        if !steer_prompts.is_empty() {
            for (seq, p) in &steer_prompts {
                let mut m = Message::user(new_id(), p.clone());
                m.synthetic = true;
                session.record(m).await;
                on_event(SessionEvent::SteerConsumed { seq: *seq });
            }
        }

        if compaction::should_compact(session) {
            match compaction::compact(session, registry, &mut *on_event).await {
                Ok(Some(summary)) => {
                    on_event(SessionEvent::TranscriptReset(session.messages.clone()));
                    on_event(SessionEvent::Compaction(summary));
                }
                Ok(None) => {}
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
        let (text, reasoning, tool_calls, usage) = turn;
        if let Some(u) = &usage {
            session.last_usage = u.clone();
        }

        let mut blocks: Vec<ContentBlock> = Vec::new();
        // Interleaved thinking: persist reasoning_content into the assistant
        // message so it's sent back on subsequent requests. Only needed on
        // tool-call turns (DeepSeek-V4 requires this and returns 400 if
        // omitted; non-tool reasoning is ignored by the API anyway).
        let it_on = session.config.interleaved_thinking.unwrap_or(true);
        if it_on && !tool_calls.is_empty() && !reasoning.is_empty() {
            blocks.push(ContentBlock::Reasoning { text: reasoning });
        }
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
            // queued input only fires when the session would otherwise go idle.
            if let Some((seq, q)) = claim_one_queued(session).await {
                let mut m = Message::user(new_id(), q);
                m.synthetic = true;
                session.record(m).await;
                on_event(SessionEvent::QueueConsumed { seq });
                continue;
            }
            on_event(SessionEvent::Done);
            break;
        }

        // ---- Tool execution: independent tool calls run concurrently so that,
        // e.g., multiple subagent (`task`) dispatches overlap instead of
        // serializing. The shared `sink` wraps the borrowed FnMut in a Mutex so
        // concurrent futures can emit events safely (each emit is a fast push).
        // Results are re-sorted by original call index so the Tool message and
        // event replay stay deterministic regardless of completion order.
        let tool_blocks: Vec<ContentBlock> = {
            let sink: Sink = Arc::new(Mutex::new(&mut *on_event));
            // Doom-loop guard, evaluated over this turn's batch.
            for tc in &tool_calls {
                let sig = format!("{}:{}", tc.name, tc.input);
                doom.push_back(sig.clone());
                if doom.len() > DOOM_THRESHOLD {
                    doom.pop_front();
                }
                if doom.len() == DOOM_THRESHOLD && doom.iter().all(|s| s == &sig) {
                    emit(
                        &sink,
                        SessionEvent::Error("doom-loop: same tool repeated 3x, stopping".into()),
                    );
                    // The assistant message carrying these `tool_use` blocks
                    // was already persisted above (line ~207). The chat API
                    // requires every `tool_use` to be followed by a matching
                    // `tool_result`; omitting them makes resuming the session
                    // fail with HTTP 400. Synthesize error results for each
                    // call so history stays well-formed.
                    let doom_blocks: Vec<ContentBlock> = tool_calls
                        .iter()
                        .map(|tc| ContentBlock::ToolResult {
                            tool_use_id: tc.id.clone(),
                            content: "doom-loop: tool execution skipped".to_string(),
                            is_error: true,
                        })
                        .collect();
                    let doom_msg = Message {
                        id: new_id(),
                        role: Role::Tool,
                        blocks: doom_blocks,
                        model: None,
                        agent: None,
                        usage: MessageUsage::default(),
                        created_at: now_ms(),
                        synthetic: false,
                    };
                    session.record(doom_msg).await;
                    return Ok(());
                }
            }
            // Announce every tool start up front, in call order.
            for tc in &tool_calls {
                emit(
                    &sink,
                    SessionEvent::ToolStart {
                        id: tc.id.clone(),
                        name: tc.name.clone(),
                        input: tc.input.clone(),
                    },
                );
            }
            let session_ref: &SessionState = session;
            let mut futs = FuturesUnordered::new();
            for (i, tc) in tool_calls.iter().enumerate() {
                let sink = Arc::clone(&sink);
                futs.push(async move {
                    tokio::task::yield_now().await;
                    let out = execute_call(tc, session_ref, registry, &sink).await;
                    (i, out)
                });
            }
            let mut results: Vec<(usize, ToolOutput)> = Vec::with_capacity(tool_calls.len());
            while let Some((i, out)) = futs.next().await {
                emit(
                    &sink,
                    SessionEvent::ToolEnd {
                        id: tool_calls[i].id.clone(),
                        name: tool_calls[i].name.clone(),
                        output: out.content.clone(),
                        is_error: out.is_error,
                    },
                );
                results.push((i, out));
                // Drain the whole batch even under cancel: breaking would drop
                // in-flight subagent futures, skipping their SubagentEnd +
                // complete_subagent_task and leaving tool_use ids without
                // results. Cancelled tools resolve fast (select! / child.cancel),
                // and the run halts at the next run_loop top-of-loop check.
            }
            results.sort_by_key(|(i, _)| *i);
            results
                .into_iter()
                .map(|(i, out)| ContentBlock::ToolResult {
                    tool_use_id: tool_calls[i].id.clone(),
                    content: out.content,
                    is_error: out.is_error,
                })
                .collect()
        };
        let tool_msg = Message {
            id: new_id(),
            role: Role::Tool,
            blocks: tool_blocks,
            model: None,
            agent: None,
            usage: MessageUsage::default(),
            created_at: now_ms(),
            synthetic: false,
        };
        session.record(tool_msg).await;
    }
    Ok(())
}

async fn run_one_llm_call(
    session: &SessionState,
    registry: &HashMap<String, ToolArc>,
    on_event: &mut (impl FnMut(SessionEvent) + Send + ?Sized),
) -> Result<(String, String, Vec<CompletedToolCall>, Option<Usage>)> {
    let system = build_system(
        &session.agent,
        &session.working_dir,
        session.skill_prompt_cloned().as_deref(),
    );
    let mut to_send = vec![system];
    to_send.extend(session.messages.iter().cloned());
    let openai_msgs = lower_messages(&to_send);

    let allowed: HashMap<String, ToolArc> = registry
        .iter()
        .filter(|(name, _)| {
            session.agent.tools.allows(name)
                && session.config.capabilities.tool_enabled(name)
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let tool_schemas = schema_for(&allowed, session.agent.kind);

    let req = ChatRequest {
        model: session.model.clone(),
        messages: openai_msgs,
        tools: tool_schemas,
        tool_choice: if allowed.is_empty() {
            None
        } else {
            Some("auto".into())
        },
        temperature: None,
        max_tokens: session.config.max_tokens,
        reasoning_effort: session.config.reasoning_effort.clone(),
    };
    let mut rx = session.client.chat_stream(req)?;
    let mut completed: Option<(String, Vec<CompletedToolCall>, Option<Usage>)> = None;
    let mut reasoning_buf = String::new();
    // True once a `Retrying` status has been shown; cleared (with an empty
    // Status event) the moment real content streams so the "↻ retry" badge
    // doesn't linger after recovery.
    let mut retried = false;
    let mut cancel_fut = std::pin::pin!(await_cancel(session));
    loop {
        tokio::select! {
            biased;
            _ = &mut cancel_fut => {
                on_event(SessionEvent::Status("interrupted".into()));
                return Ok((String::new(), String::new(), Vec::new(), None));
            }
            ev = rx.recv() => {
                let ev = match ev { Some(ev) => ev, None => break };
                match ev {
                    LlmEvent::TextDelta(t) => {
                        if retried {
                            retried = false;
                            on_event(SessionEvent::Status(String::new()));
                        }
                        on_event(SessionEvent::TextDelta(t));
                    }
                    LlmEvent::ReasoningDelta(r) => {
                        if retried {
                            retried = false;
                            on_event(SessionEvent::Status(String::new()));
                        }
                        reasoning_buf.push_str(&r);
                        on_event(SessionEvent::ReasoningDelta(r));
                    }
                    LlmEvent::ToolCallStart { .. } | LlmEvent::ToolCallDelta { .. } => {}
                    LlmEvent::Completed { text, tool_calls, usage } => {
                        if retried {
                            retried = false;
                            on_event(SessionEvent::Status(String::new()));
                        }
                        completed = Some((text, tool_calls, usage));
                    }
                    LlmEvent::Retrying { attempt, max } => {
                        retried = true;
                        on_event(SessionEvent::Status(format!(
                            "\u{21bb} retry {attempt}/{max}"
                        )));
                    }
                    LlmEvent::Error(e) => return Err(anyhow!(e)),
                }
            }
        }
    }
    let (text, tool_calls, usage) =
        completed.ok_or_else(|| anyhow!("stream ended without completion"))?;
    Ok((text, reasoning_buf, tool_calls, usage))
}

async fn execute_call(
    tc: &CompletedToolCall,
    session: &SessionState,
    registry: &HashMap<String, ToolArc>,
    sink: &Sink<'_>,
) -> ToolOutput {
    if tc.name == "task" {
        return run_subagent(tc.input.clone(), tc.id.clone(), session, registry, sink).await;
    }
    // Plan-mode bash write guard: classify the command and block mutating
    // operations, returning a descriptive error to the model so it can adapt.
    if tc.name == "bash" && session.agent.kind == AgentKind::Plan {
        let cmd = tc
            .input
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if let crate::bash_guard::BashVerdict::WriteBlocked(reason) =
            crate::bash_guard::classify(cmd)
        {
            return ToolOutput::err(format!(
                "Blocked in plan mode: this bash command modifies state ({reason}). \
                 Plan mode is read-only. To make changes, switch to act mode (Alt+Tab)."
            ));
        }
    }
    let ctx = ToolContext {
        session_id: session.id.clone(),
        message_id: tc.id.clone(),
        agent: session.agent.name.clone(),
        working_dir: session.working_dir.clone(),
        max_output: MAX_OUTPUT,
        proxy: session.config.network.proxy.clone(),
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
    sink: &Sink<'_>,
) -> ToolOutput {
    let prompt = input
        .get("prompt")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if prompt.is_empty() {
        return ToolOutput::err("task requires a prompt");
    }
    let kind = input
        .get("subagent_type")
        .and_then(|v| v.as_str())
        .unwrap_or("explore")
        .to_string();
    // Plan mode may only spawn read-only subagents: 'explore' (filesystem) and
    // 'tools' (browser fetch/search + computer-use are read-only w.r.t. the
    // repo). 'build' stays rejected so the model is never told it exists.
    if parent.agent.kind == AgentKind::Plan && !matches!(kind.as_str(), "explore" | "tools") {
        return ToolOutput::err(format!(
            "Unknown subagent_type '{kind}'. Valid options: 'explore' (read-only) or 'tools' (browser/computer-use)."
        ));
    }
    let agent = match resolve_agent(&kind) {
        Some(a) => a,
        None => {
            return ToolOutput::err(format!(
                "Unknown subagent_type '{kind}'. Valid options: 'explore' (read-only), 'build' (full tools), or 'tools' (browser/computer-use)."
            ));
        }
    };
    let child_session_id = format!("sub-{}", new_id());
    let preview: String = prompt.chars().take(80).collect();
    emit(
        sink,
        SessionEvent::SubagentStart {
            id: call_id.clone(),
            kind: kind.clone(),
            prompt: preview,
            child_session_id: child_session_id.clone(),
        },
    );

    let mut child = SessionState::new(
        child_session_id.clone(),
        agent,
        parent.config.clone(),
        parent.client.clone(),
        parent.working_dir.clone(),
    );
    // Propagate the parent's cancellation token so a double-Esc also stops a
    // running subagent at its next turn boundary.
    child.cancel = parent.cancel.clone();

    // Attach the parent's store so the child's messages persist to libsql
    // under its own session id. Also record the parent-child relationship.
    if let Some(store) = &parent.store {
        child = child.with_store(store.clone());
        // Seed the child session row so the FK on subagent_tasks resolves.
        let _ = store
            .create_session(&opencoder_store::SessionMeta {
                id: child_session_id.clone(),
                title: Some(prompt.chars().take(60).collect()),
                agent: Some(kind.clone()),
                model: Some(parent.config.model_id().to_string()),
                workdir_hash: None,
                created_at: now_ms(),
                updated_at: now_ms(),
                summary: None,
                summary_seq: None,
                handoff_seq: None,
                handoff_plan: None,
                skill: None,
            })
            .await;
        // Mark the child session as already created so persist() doesn't
        // auto-create a duplicate row with conflicting metadata.
        child = child.mark_session_created();
        let parent_msg_id = parent
            .messages
            .iter()
            .rev()
            .find(|m| m.role == Role::Assistant)
            .map(|m| m.id.clone());
        let rec = SubagentTaskRecord {
            task_id: call_id.clone(),
            parent_session_id: parent.id.clone(),
            child_session_id: child_session_id.clone(),
            parent_message_id: parent_msg_id,
            agent: kind.clone(),
            prompt: prompt.clone(),
            result: None,
            status: SubagentStatus::Running,
            ok: None,
            started_at: now_ms(),
            completed_at: None,
        };
        let _ = store.create_subagent_task(&rec).await;
    }

    // Forward child events to the parent sink and persist them for replay.
    let mut child_chars = String::new();
    let mut child_tools: u32 = 0;
    let child_store = parent.store.clone();
    let child_id_for_cb = child_session_id.clone();
    let summary_chars = &mut child_chars;
    let tool_count = &mut child_tools;
    let parent_sink = Arc::clone(sink);
    let call_id_for_cb = call_id.clone();
    let has_store = child_store.is_some();
    // Incremental child-event persistence: a single flusher task drains an
    // mpsc channel and awaits `append_event` per record in emission order (one
    // consumer → DB seq stays aligned with emission order). Events reach the DB
    // as they are produced, so a hard interruption mid-subagent leaves partial
    // progress persisted (reconstruct_child_view reads events_after(child, 0))
    // instead of losing everything. The flusher is awaited before return so a
    // normal completion flushes 100% of buffered events.
    let (ev_tx, mut ev_rx) = tokio::sync::mpsc::channel::<SessionEventRecord>(256);
    let flush_store = child_store.clone();
    let flusher = tokio::spawn(async move {
        while let Some(rec) = ev_rx.recv().await {
            if let Some(store) = &flush_store {
                if let Err(e) = store.append_event(&rec).await {
                    tracing::warn!(error = %e, "subagent: failed to persist child event");
                }
            }
        }
    });
    let res = Box::pin(run_with_registry(
        &mut child,
        prompt.clone(),
        registry,
        move |cev| {
            // Incremental persist: push to the ordered flusher channel. The
            // callback is sync (cannot await), so try_send; a full/closed
            // channel is logged and the single event dropped rather than
            // blocking the run.
            if has_store {
                let rec = SessionEventRecord {
                    session_id: child_id_for_cb.clone(),
                    kind: cev.coarse_kind(),
                    payload: serde_json::to_value(&cev).unwrap_or(serde_json::Value::Null),
                    ts: now_ms(),
                    seq: None,
                    sse_kind: Some(cev.sse_kind().to_string()),
                };
                if let Err(e) = ev_tx.try_send(rec) {
                    tracing::warn!(error = %e, "subagent: child event channel full/closed, dropping event");
                }
            }
            match &cev {
                SessionEvent::ToolStart { .. } => *tool_count += 1,
                SessionEvent::TextDelta(t) if summary_chars.len() < 240 => {
                    summary_chars.push_str(t);
                }
                _ => {}
            }
            emit(
                &parent_sink,
                SessionEvent::SubagentChild {
                    id: call_id_for_cb.clone(),
                    ev: Box::new(cev),
                },
            );
        },
    ))
    .await;

    // The callback owned `ev_tx`; once `run_with_registry` returns the closure
    // is dropped, closing the channel so the flusher drains remaining events
    // and exits. Await it so this function returns only after every event is
    // durably persisted.
    let _ = flusher.await;

    let ok = res.is_ok();
    let text = child
        .messages
        .iter()
        .rev()
        .find(|m| m.role == Role::Assistant)
        .map(|m| m.text())
        .unwrap_or_default();

    // Record completion: prompt + result in libsql.
    if let Some(store) = &parent.store {
        let _ = store.complete_subagent_task(&call_id, &text, ok).await;
    }

    let summary_preview: String = if child_chars.is_empty() {
        text.chars().take(120).collect()
    } else {
        child_chars.chars().take(120).collect()
    };
    emit(
        sink,
        SessionEvent::SubagentEnd {
            id: call_id.clone(),
            ok,
            summary: format!("({} tool calls) {}", child_tools, summary_preview),
        },
    );
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
/// return their `(row seq, prompt)` pairs to be appended to history. The row
/// seq is the `session_inputs` primary key -- the same identity `admit_input`
/// returns and the TUI stores in its `steer_items` mirror -- so a
/// `SteerConsumed` event lets the TUI drop the row by identity instead of
/// leaving a stale `steer` row until `Done`. This is NOT the per-session
/// `admitted_seq` (a different column scoped per session). No-op when no store
/// is attached or none pending. Idempotent (promote only touches NULL
/// promoted_seq).
async fn claim_steers(session: &mut SessionState) -> Vec<(i64, String)> {
    let store = match session.store.clone() {
        Some(s) => s,
        None => return Vec::new(),
    };
    let pending = match store
        .pending_inputs(&session.id, opencoder_store::Delivery::Steer)
        .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "claim_steers: pending_inputs failed");
            return Vec::new();
        }
    };
    if pending.is_empty() {
        return Vec::new();
    }
    let max_seq = pending.iter().map(|i| i.admitted_seq).max().unwrap_or(0);
    // `promote_inputs` returns the promoted rows' PK seqs (`SELECT seq ...
    // ORDER BY admitted_seq ASC`) -- the same ordering `pending_inputs` uses,
    // so the two vectors align 1:1. Pair each PK with its prompt rather than
    // using `admitted_seq`, so `SteerConsumed` carries the identity the TUI
    // stored via `admit_input`'s return value.
    let promoted_seqs = match store
        .promote_inputs(&session.id, max_seq, opencoder_store::Delivery::Steer)
        .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "claim_steers: promote_inputs failed");
            return Vec::new();
        }
    };
    pending
        .into_iter()
        .zip(promoted_seqs)
        .map(|(i, seq)| (seq, i.prompt))
        .collect()
}

/// Claim exactly one queued input at idle. Returns its (row seq, prompt), or None.
async fn claim_one_queued(session: &mut SessionState) -> Option<(i64, String)> {
    let store = session.store.clone()?;
    match store.claim_next_queue(&session.id).await {
        Ok(Some((seq, input))) => Some((seq, input.prompt)),
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
    config: opencoder_core::Config,
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

#[cfg(test)]
mod from_sse_tests {
    use super::*;

    /// `from_sse` is the exact inverse of `sse_kind()` + `sse_data()` for every
    /// variant EXCEPT `TranscriptReset`, whose payload is `{}` on the wire
    /// (the rebuilt message list cannot be carried over SSE and must be
    /// re-fetched). Pin both the roundtrip and that documented lossiness.
    #[test]
    fn from_sse_roundtrips_all_variants() {
        let cases: Vec<SessionEvent> = vec![
            SessionEvent::TextDelta("hi".into()),
            SessionEvent::ReasoningDelta("think".into()),
            SessionEvent::ToolStart {
                id: "t1".into(),
                name: "bash".into(),
                input: serde_json::json!({"command": "ls"}),
            },
            SessionEvent::ToolEnd {
                id: "t1".into(),
                name: "bash".into(),
                output: "done".into(),
                is_error: false,
            },
            SessionEvent::ToolEnd {
                id: "t2".into(),
                name: "bash".into(),
                output: "boom".into(),
                is_error: true,
            },
            SessionEvent::AgentSwitch("plan".into()),
            SessionEvent::Compaction("summary".into()),
            SessionEvent::Status("running".into()),
            SessionEvent::SubagentStart {
                id: "s1".into(),
                kind: "explore".into(),
                prompt: "find x".into(),
                child_session_id: "child-1".into(),
            },
            SessionEvent::SubagentEnd {
                id: "s1".into(),
                ok: true,
                summary: "found".into(),
            },
            SessionEvent::SubagentChild {
                id: "s1".into(),
                ev: Box::new(SessionEvent::TextDelta("child text".into())),
            },
            SessionEvent::PlanHandoff("# plan".into()),
            SessionEvent::TranscriptReset(vec![Message::assistant("m1")]),
            SessionEvent::QueueConsumed { seq: 7 },
            SessionEvent::SteerConsumed { seq: 9 },
            SessionEvent::Done,
            SessionEvent::Error("kaboom".into()),
        ];
        let mut kinds: Vec<&str> = cases.iter().map(|e| e.sse_kind()).collect();
        kinds.sort();
        kinds.dedup();
        assert_eq!(kinds.len(), 16, "expected all 16 unique kinds, got {kinds:?}");

        for ev in &cases {
            let kind = ev.sse_kind();
            let data = ev.sse_data();
            let back = SessionEvent::from_sse(kind, data.clone())
                .unwrap_or_else(|| panic!("from_sse returned None for kind={kind} data={data}"));
            if matches!(ev, SessionEvent::TranscriptReset(_)) {
                // documented lossiness: no messages on the wire
                assert!(matches!(back, SessionEvent::TranscriptReset(ref v) if v.is_empty()));
            } else {
                assert_eq!(
                    serde_json::to_string(&back).unwrap(),
                    serde_json::to_string(ev).unwrap(),
                    "roundtrip mismatch for kind={kind}"
                );
            }
        }
    }

    #[test]
    fn from_sse_unknown_kind_is_none() {
        assert!(SessionEvent::from_sse("no_such_kind", serde_json::json!({})).is_none());
    }

    #[test]
    fn from_sse_missing_field_is_none() {
        // tool_start without the required `name` field
        assert!(SessionEvent::from_sse("tool_start", serde_json::json!({"id":"x"})).is_none());
    }
}
