use std::collections::{HashMap, HashSet, VecDeque};
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
use llm_call::{core_usage, run_one_llm_call};
pub(crate) use steer::await_cancel;
use steer::{claim_one_queued, claim_steers, is_turn_cancelled, reset_turn_cancel};

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
    mut user_text: String,
    images: Vec<String>,
    registry: &HashMap<String, ToolArc>,
    on_event: impl FnMut(SessionEvent) + Send,
) -> Result<()> {
    let mut on_event = on_event;
    // Control commands (/act, /plan, /act_clear_context) take effect
    // immediately without consuming an LLM turn. Short-circuit when the idle
    // prompt is one so CLI/Web/TUI free-text idle use skips run_loop entirely.
    if let Some(cmd) = crate::control_cmd::parse(&user_text) {
        crate::control_cmd::apply(session, &cmd, &mut on_event).await?;
        on_event(SessionEvent::Done);
        return Ok(());
    }
    // Replay any subagent tasks left cancelled from a prior interrupted run
    // BEFORE the user's new input enters the loop: resume each cancelled child,
    // run it to completion, backfill the parent tool_result, and flip the task
    // to Completed. The model then sees [user input + subagent result] together
    // and the interrupted call is transparently resumed. No-op for children
    // (they hold no `task` tool, so they have no subagent tasks).
    // The TUI passes prompts directly as `user_text` (not via store Delivery),
    // so we check that here: when the user typed new input, cancelled subagents
    // should be abandoned rather than silently replayed.
    let has_new_input = !user_text.is_empty() || !images.is_empty();
    crate::resume::replay_cancelled_tasks(session, has_new_input).await;
    // In-process safety net for well-formed history: any `tool_use` id left
    // dangling by a prior interrupted batch (and not replayable via
    // replay_cancelled_tasks / resume_and_replay) is answered with a synthetic
    // error tool_result, so the next LLM request cannot hit the provider's
    // "unanswered tool_call" HTTP 400. Idempotent; runs before the new user
    // input is recorded so the synthesized message lands at the transcript end.
    crate::dangling_tools::reconcile_dangling_tool_uses(session).await;
    // A non-empty prompt records a real user message. An empty prompt means
    // "drain mode": the web drain relies on admitted steers/queues being
    // claimed at turn boundaries to supply the actual user input, and the web
    // has no skill support (`skill_prompt` is `None`). When an active skill is
    // set and the user submitted no text (pure-skill submit or image-only),
    // inject a synthetic trigger so the model records a user turn and acts on
    // the skill body in the system prompt instead of treating the input
    // passively. For text-bearing turns the user's own words drive execution.
    let has_skill = session.skill_prompt_cloned().is_some();
    let has_text = !user_text.trim().is_empty();
    let has_images = !images.is_empty();
    if has_text || has_images {
        session.maybe_tag_plan_prompt(&mut user_text);
        let user = Message::user_with_images(new_id(), user_text, &images);
        session.record(user).await;
    }
    if has_skill && !has_text {
        let mut msg = Message::user(
            new_id(),
            "The active skill is now in effect. Begin executing it now.",
        );
        msg.synthetic = true;
        session.record(msg).await;
    }
    run_loop(session, registry, &mut on_event).await?;
    // Autopilot: after the initial task completes, hand control to the
    // PLAN -> ACT -> VERIFY loop so the agent self-drives toward the goal.
    if session.config.autopilot.enabled {
        crate::autopilot::drive(session, registry, &mut on_event).await?;
    }
    Ok(())
}

/// Deduplicate consecutive bash-timeout tool results.
///
/// When bash times out repeatedly (e.g. the model keeps running long commands
/// across turns), only the first timeout's full message is kept \u{2014} subsequent
/// ones are replaced with the first's content (same PID, same output file
/// path, so the model reads the same file regardless). A non-timeout bash
/// result resets the consecutive count. Non-bash tool calls do NOT reset it
/// (they run independently and don't affect the bash timeout streak).
///
/// `first` persists across turns (it lives in `run_loop`'s scope) so the
/// dedup applies across turn boundaries, not just within a single batch.
fn dedup_consecutive_bash_timeouts(
    tool_calls: &[opencoder_llm::CompletedToolCall],
    results: &mut [(usize, ToolOutput)],
    first: &mut Option<String>,
) {
    for (i, out) in results.iter_mut() {
        let is_bash = tool_calls.get(*i).is_some_and(|tc| tc.name == "bash");
        if is_bash
            && out
                .content
                .starts_with(crate::tools::bash::BASH_TIMEOUT_MARKER)
        {
            if first.is_some() {
                out.content = first.clone().unwrap();
            } else {
                *first = Some(out.content.clone());
            }
        } else if is_bash {
            *first = None;
        }
    }
}

pub(crate) async fn run_loop(
    session: &mut SessionState,
    registry: &HashMap<String, ToolArc>,
    on_event: &mut (dyn FnMut(SessionEvent) + Send),
) -> Result<()> {
    let mut doom: VecDeque<String> = VecDeque::new();
    let mut tool_failures: crate::tool_guard::FailureMap = HashMap::new();
    // Tracks the first bash-timeout output in a consecutive run so that
    // subsequent timeouts can be deduplicated (same PID / output file).
    let mut bash_timeout_first: Option<String> = None;

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
                on_event(SessionEvent::SteerConsumed { seq: *seq });
                // Defensive: a steered control command is applied immediately
                // and NOT recorded as a user message, so "/plan" never leaks
                // to the LLM as literal text.
                if let Some(cmd) = crate::control_cmd::parse(p) {
                    crate::control_cmd::apply(session, &cmd, &mut *on_event).await?;
                    continue;
                }
                let mut text = p.clone();
                session.maybe_tag_plan_prompt(&mut text);
                let mut m = Message::user_with_images(new_id(), text, imgs);
                m.synthetic = true;
                session.record(m).await;
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
        // Turn-level interrupt (subagent steer): the LLM call was cut short by
        // a turn-cancel. Don't record the empty assistant message — just reset
        // the token and continue to the top of the loop where claim_steers
        // absorbs the pending steer.
        if is_turn_cancelled(session) {
            reset_turn_cancel(session);
            continue;
        }
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
            // Idle boundary: drain FIFO queued follow-ups until empty.
            // Control commands (/act, /plan, /act_clear_context) are applied
            // immediately without an LLM turn, so multiple can be drained in
            // sequence. A real prompt breaks the inner loop so the outer loop
            // processes it; once its turn completes the next idle boundary
            // claims the next queued item until the queue is empty (Done).
            let mut got_real_prompt = false;
            loop {
                if let Some((seq, mut q, imgs)) = claim_one_queued(session).await {
                    on_event(SessionEvent::QueueConsumed { seq });
                    if let Some(cmd) = crate::control_cmd::parse(&q) {
                        crate::control_cmd::apply(session, &cmd, &mut *on_event).await?;
                        continue; // drain next queued item, no LLM turn
                    }
                    // Real prompt: record it and let the outer loop process it.
                    session.maybe_tag_plan_prompt(&mut q);
                    let mut m = Message::user_with_images(new_id(), q, &imgs);
                    m.synthetic = true;
                    session.record(m).await;
                    got_real_prompt = true;
                    break;
                }
                // Queue empty: go idle.
                on_event(SessionEvent::Done);
                break;
            }
            if got_real_prompt {
                continue; // outer loop: LLM processes the recorded prompt
            }
            break; // outer loop: idle (Done emitted)
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
                        SessionEvent::Error(format!(
                            "doom-loop: same tool repeated {}x, stopping",
                            DOOM_THRESHOLD
                        )),
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
                    return Err(anyhow!("doom-loop: same tool repeated {}x", DOOM_THRESHOLD));
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
            // Deduplicate consecutive bash-timeout results: only the first
            // timeout in a streak shows its full message; subsequent ones
            // reuse the first content (same PID, same output file).
            dedup_consecutive_bash_timeouts(&tool_calls, &mut results, &mut bash_timeout_first);
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
        // Turn-level interrupt (subagent steer): the tool batch was
        // interrupted. Record the tool results normally for history
        // integrity, then continue to absorb pending steers.
        let turn_was_interrupted = is_turn_cancelled(session);
        if turn_was_interrupted {
            reset_turn_cancel(session);
        }
        // Hard cancel mid-tool-batch: record every non-replayable tool result
        // so the transcript stays well-formed — dropping the whole tool message
        // (the old behavior) left its `tool_use` ids dangling and provoked a
        // provider HTTP 400 on the next turn. `task` tool_use ids whose
        // subagent is still replayable (Running/Cancelled in the store) stay
        // dangling on purpose: their results are backfilled by
        // replay_cancelled_tasks / resume_and_replay on the next user turn.
        // In-process continuation additionally hits the
        // reconcile_dangling_tool_uses safety net in run_with_registry.
        if session
            .cancel
            .as_ref()
            .map(|c| c.is_cancelled())
            .unwrap_or(false)
        {
            let replayable: HashSet<String> = match session.store.clone() {
                Some(store) => {
                    let records = store
                        .list_subagent_tasks(&session.id)
                        .await
                        .unwrap_or_default();
                    crate::dangling_tools::replayable_task_ids_from_records(&records)
                }
                // Store-less session: nothing can be replayed, so the batch's
                // results (task included) are all recorded.
                None => HashSet::new(),
            };
            let non_replayable: Vec<ContentBlock> = tool_blocks
                .into_iter()
                .filter(|b| match b {
                    ContentBlock::ToolResult { tool_use_id, .. } => {
                        !replayable.contains(tool_use_id)
                    }
                    _ => true,
                })
                .collect();
            if !non_replayable.is_empty() {
                let tool_msg = Message {
                    id: new_id(),
                    role: Role::Tool,
                    blocks: non_replayable,
                    model: None,
                    agent: None,
                    usage: MessageUsage::default(),
                    created_at: now_ms(),
                    synthetic: false,
                };
                session.record(tool_msg).await;
            }
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

        if turn_was_interrupted {
            continue;
        }

        // Tool-failure threshold: if any tool hit the consecutive-failure
        // limit, abort the turn to break the retry loop.
        if failure_tripped {
            let detail = crate::tool_guard::worst(&tool_failures)
                .map(|(n, c)| format!("'{n}' failed {c}x consecutively"))
                .unwrap_or_else(|| "threshold reached".into());
            on_event(SessionEvent::Error(format!(
                "tool-failure guard: {detail}, stopping"
            )));
            return Err(anyhow!("tool-failure guard: {detail}"));
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

#[cfg(test)]
mod dedup_tests {
    use super::dedup_consecutive_bash_timeouts;
    use opencoder_core::ToolOutput;
    use opencoder_llm::CompletedToolCall;
    use serde_json::json;

    fn bash_tc(id: &str) -> CompletedToolCall {
        CompletedToolCall {
            id: id.into(),
            name: "bash".into(),
            input: json!({}),
        }
    }

    fn other_tc(name: &str, id: &str) -> CompletedToolCall {
        CompletedToolCall {
            id: id.into(),
            name: name.into(),
            input: json!({}),
        }
    }

    fn timeout_output(pid: u32) -> ToolOutput {
        ToolOutput {
            content: format!(
                "[bash-timeout: command timed out after 1s \u{2014} moved to background]\n\
                 pid: {pid}\noutput: /tmp/opencoder_bg_{pid}.output\n\n"
            ),
            is_error: false,
            images: vec![],
        }
    }

    fn normal_output(text: &str) -> ToolOutput {
        ToolOutput::ok(text)
    }

    #[test]
    fn first_timeout_stored_subsequent_replaced() {
        let tool_calls = vec![bash_tc("1"), bash_tc("2")];
        let mut results = vec![(0, timeout_output(100)), (1, timeout_output(200))];
        let mut first = None;
        dedup_consecutive_bash_timeouts(&tool_calls, &mut results, &mut first);
        assert_eq!(
            results[0].1.content, results[1].1.content,
            "second timeout content must match first"
        );
        assert!(results[0].1.content.contains("pid: 100"));
        assert!(
            results[1].1.content.contains("pid: 100"),
            "second must be replaced with first content (pid 100)"
        );
    }

    #[test]
    fn non_timeout_bash_resets_count() {
        let tool_calls = vec![bash_tc("1"), bash_tc("2"), bash_tc("3")];
        let mut results = vec![
            (0, timeout_output(100)),
            (1, normal_output("done")),
            (2, timeout_output(300)),
        ];
        let mut first = None;
        dedup_consecutive_bash_timeouts(&tool_calls, &mut results, &mut first);
        assert!(results[0].1.content.contains("pid: 100"));
        assert!(results[1].1.content.contains("done"));
        assert!(
            results[2].1.content.contains("pid: 300"),
            "third timeout must have own content after reset"
        );
    }

    #[test]
    fn non_bash_tool_does_not_reset_count() {
        let tool_calls = vec![bash_tc("1"), other_tc("edit", "2"), bash_tc("3")];
        let mut results = vec![
            (0, timeout_output(100)),
            (1, normal_output("edited")),
            (2, timeout_output(300)),
        ];
        let mut first = None;
        dedup_consecutive_bash_timeouts(&tool_calls, &mut results, &mut first);
        assert!(results[0].1.content.contains("pid: 100"));
        assert!(results[1].1.content.contains("edited"));
        assert!(
            results[2].1.content.contains("pid: 100"),
            "third timeout must reuse first content (non-bash didn't reset)"
        );
    }

    #[test]
    fn first_persists_across_batches() {
        let tool_calls_a = vec![bash_tc("1")];
        let mut results_a = vec![(0, timeout_output(100))];
        let mut first = None;
        dedup_consecutive_bash_timeouts(&tool_calls_a, &mut results_a, &mut first);

        let tool_calls_b = vec![bash_tc("2")];
        let mut results_b = vec![(0, timeout_output(200))];
        dedup_consecutive_bash_timeouts(&tool_calls_b, &mut results_b, &mut first);

        assert!(
            results_b[0].1.content.contains("pid: 100"),
            "second-batch timeout must reuse first-batch content"
        );
    }
}
