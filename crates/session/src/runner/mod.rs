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
use opencoder_store::{SessionEventRecord, SubagentStatus, SubagentTaskRecord};
use serde_json::Value;

use crate::compaction;
use crate::prompt::build_system;
use crate::tools::{registry as build_registry, schema_for};
use crate::SessionState;


mod event;
mod subagent;

pub use event::SessionEvent;
use event::{Sink, MAX_OUTPUT, DOOM_THRESHOLD};
use subagent::run_subagent;

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
    run_with_registry(session, user_text, Vec::new(), &registry, on_event).await
}

/// Like [`run`] but attaches `images` (data URIs or URLs) as `Image` content
/// blocks to the first user message, enabling multimodal/vision prompts from
/// the headless CLI (`opencode run "..." --image ./a.png`).
pub async fn run_with_images(
    session: &mut SessionState,
    user_text: String,
    images: Vec<String>,
    on_event: impl FnMut(SessionEvent) + Send,
) -> Result<()> {
    let registry = build_registry();
    run_with_registry(session, user_text, images, &registry, on_event).await
}

pub async fn run_with_registry(
    session: &mut SessionState,
    user_text: String,
    images: Vec<String>,
    registry: &HashMap<String, ToolArc>,
    on_event: impl FnMut(SessionEvent) + Send,
) -> Result<()> {
    let mut on_event = on_event;
    // Replay any subagent tasks left cancelled from a prior interrupted run
    // BEFORE the user's new input enters the loop: resume each cancelled child,
    // run it to completion, backfill the parent tool_result, and flip the task
    // to Completed. The model then sees [user input + subagent result] together
    // and the interrupted call is transparently resumed. No-op for children
    // (they hold no `task` tool, so they have no subagent tasks).
    crate::resume::replay_cancelled_tasks(session).await;
    // A non-empty prompt records a real user message. An empty prompt means
    // "drain mode": the web drain relies on admitted steers/queues being
    // claimed at turn boundaries to supply the actual user input, and the web
    // has no skill support (`skill_prompt` is `None`). But for skill-only
    // submits (empty prompt with an active skill), inject a synthetic trigger
    // message so the model records a user turn and acts on the skill body in
    // the system prompt instead of treating it passively.
    if !user_text.trim().is_empty() {
        let user = Message::user_with_images(new_id(), user_text, &images);
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
    let mut tool_failures: crate::tool_guard::FailureMap = HashMap::new();

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
            for (seq, p, imgs) in &steer_prompts {
                let mut m = Message::user_with_images(new_id(), p.clone(), imgs);
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
            if let Some((seq, q, imgs)) = claim_one_queued(session).await {
                let mut m = Message::user_with_images(new_id(), q, &imgs);
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
        let mut failure_tripped = false;
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
            // Tool-failure guard: track consecutive failures per tool name
            // and apply exponential backoff before continuing.
            {
                let tg = &session.config.tool_guard;
                if tg.max_consecutive_failures > 0 {
                    let mut max_delay = std::time::Duration::ZERO;
                    for &(i, ref out) in &results {
                        let (tripped, delay) = crate::tool_guard::record(
                            &mut tool_failures,
                            &tool_calls[i].name,
                            out.is_error,
                            tg,
                        );
                        if tripped {
                            failure_tripped = true;
                        }
                        if delay > max_delay {
                            max_delay = delay;
                        }
                    }
                    if !max_delay.is_zero() {
                        tokio::time::sleep(max_delay).await;
                    }
                }
            }
            results
                .into_iter()
                .map(|(i, out)| ContentBlock::ToolResult {
                    tool_use_id: tool_calls[i].id.clone(),
                    content: out.content,
                    is_error: out.is_error,
                })
                .collect()
        };
        // If interrupted mid-tool-batch, drop the tool message entirely so a
        // cancelled subagent's `task` tool_use stays dangling (replayed on the
        // next user turn by run_with_registry). Other interrupted tool_uses are
        // reconciled to error results by resume()'s dangling-tool_use path.
        if session
            .cancel
            .as_ref()
            .map(|c| c.is_cancelled())
            .unwrap_or(false)
        {
            on_event(SessionEvent::Status("interrupted".into()));
            break;
        }
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

        // Tool-failure threshold: if any tool hit the consecutive-failure
        // limit, abort the turn to break the retry loop.
        if failure_tripped {
            let detail = crate::tool_guard::worst(&tool_failures)
                .map(|(n, c)| format!("'{n}' failed {c}x consecutively"))
                .unwrap_or_else(|| "threshold reached".into());
            on_event(SessionEvent::Error(format!(
                "tool-failure guard: {detail}, stopping"
            )));
            break;
        }
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
        &session.config.capabilities,
    );
    let mut to_send = vec![system];
    to_send.extend(session.messages.iter().cloned());
    let openai_msgs = lower_messages(&to_send);

    let skill_body = session.skill_prompt_cloned();
    let unlocked = crate::tools::latent::unlocked_from_body(skill_body.as_deref());
    let allowed: HashMap<String, ToolArc> = registry
        .iter()
        .filter(|(name, _)| {
            session.agent.tools.allows(name)
                && session.config.capabilities.tool_enabled(name)
                && (!crate::tools::latent::is_latent_tool(name.as_str())
                    || unlocked.contains(name.as_str()))
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let tool_schemas = schema_for(&allowed, session.agent.kind, &session.config.capabilities);

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
        cache_salt: crate::cache_salt_for(session),
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

fn core_usage(u: &Usage) -> MessageUsage {
    MessageUsage {
        input_tokens: u.input_tokens,
        output_tokens: u.output_tokens,
        total_tokens: u.total_tokens,
        cache_read_tokens: u.cache_read_tokens,
        cache_creation_tokens: u.cache_creation_tokens,
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
async fn claim_steers(session: &mut SessionState) -> Vec<(i64, String, Vec<String>)> {
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
        .map(|(i, seq)| (seq, i.prompt, i.images.clone()))
        .collect()
}

/// Claim exactly one queued input at idle. Returns its (row seq, prompt), or None.
async fn claim_one_queued(session: &mut SessionState) -> Option<(i64, String, Vec<String>)> {
    let store = session.store.clone()?;
    match store.claim_next_queue(&session.id).await {
        Ok(Some((seq, input))) => Some((seq, input.prompt, input.images.clone())),
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
