use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use futures::stream::{FuturesUnordered, StreamExt};
use opencoder_core::{
    message::now_ms, resolve_agent, AgentKind, ContentBlock, Message, MessageUsage, Role, ToolArc,
    ToolOutput,
};
use opencoder_llm::ChatStream;
use opencoder_store::{SessionEventRecord, SubagentStatus, SubagentTaskRecord};
use serde_json::Value;

use crate::compaction;
use crate::tools::registry as build_registry;
use crate::SessionState;

mod event;
mod execute;
mod llm_call;
mod steer;
mod subagent;

pub use event::SessionEvent;
use event::{Sink, DOOM_THRESHOLD};
use execute::execute_call;
pub(crate) use execute::DEFAULT_TOOL_TIMEOUT;
use llm_call::{core_usage, run_one_llm_call};
pub(crate) use steer::await_cancel;
use steer::{claim_one_queued, claim_steers};

/// Emit an event through the shared sink. Best-effort: a poisoned mutex (only
/// possible on panic inside a closure) drops the event rather than propagating.
fn emit(sink: &Sink<'_>, ev: SessionEvent) {
    if let Ok(mut g) = sink.lock() {
        // g: MutexGuard<&mut (dyn FnMut + Send)>; deref to the inner closure
        // reference and call it.
        (**g)(ev);
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
        // Streamline the completed assistant text before it is persisted and
        // re-sent as context. The live TextDelta stream already delivered the
        // verbatim original to the UI, so this only trims the stored +
        // future-context copy (fenced code is preserved verbatim).
        let text = crate::streamline::streamline(&text, &session.config.output_streamline);
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
                            images: Vec::new(),
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
                        images: out.images.clone(),
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
                    images: out.images,
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
