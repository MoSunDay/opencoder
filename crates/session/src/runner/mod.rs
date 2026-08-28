use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use futures::stream::{FuturesUnordered, StreamExt};
use futures::FutureExt;
use opencoder_core::{
    message::now_ms, resolve_agent, AgentKind, ContentBlock, Message, MessageUsage, Role, ToolArc,
    ToolOutput,
};
use opencoder_llm::ChatStream;
use opencoder_store::{SessionEventRecord, SubagentStatus, SubagentTaskRecord};
use serde_json::Value;
use std::panic::AssertUnwindSafe;

use crate::compaction;
// run_loop_one_shot: every primary run ends with the one-shot skill clear.
use crate::skill_lifecycle::run_loop_one_shot;
use crate::SessionState;
use registry::build_full_registry;

mod dedup;
mod drain;
mod event;
mod execute;
mod input_recovery;
mod llm_call;
mod registry;
mod steer;
mod subagent;
#[cfg(test)]
#[path = "test_fixtures.rs"]
mod test_fixtures;

pub use event::SessionEvent;
use event::{Sink, DOOM_THRESHOLD};

use drain::{
    drain_mode_step, entry_drain_mode, idle_drain, reabsorb_tail, DrainModeAction, IdleAction,
    MAX_CONSUME_STREAK,
};
use execute::{execute_call, panic_message};
use llm_call::{core_usage, run_one_llm_call};
pub(crate) use steer::await_cancel;
use steer::{
    apply_steer_batch, claim_steers, is_turn_cancelled, reset_turn_cancel, SteerApplyOutcome,
};

/// Emit an event through the shared sink. Best-effort: a poisoned mutex (only
/// possible on panic inside a closure) drops the event rather than propagating.
fn emit(sink: &Sink<'_>, ev: SessionEvent) {
    if let Ok(mut g) = sink.lock() {
        // g: MutexGuard<&mut (dyn FnMut + Send)>; deref to the inner closure
        // reference and call it.
        (**g)(ev);
    } else {
        tracing::warn!(event = ?ev, "emit: sink mutex poisoned, event dropped");
    }
}

pub async fn run(
    session: &mut SessionState,
    user_text: String,
    on_event: impl FnMut(SessionEvent) + Send,
) -> Result<()> {
    let registry = build_full_registry(session).await;
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
    let registry = build_full_registry(session).await;
    run_with_registry(session, user_text, images, &registry, on_event).await
}

pub async fn run_with_registry(
    session: &mut SessionState,
    mut user_text: String,
    mut images: Vec<String>,
    registry: &HashMap<String, ToolArc>,
    on_event: impl FnMut(SessionEvent) + Send,
) -> Result<()> {
    let mut on_event = on_event;
    // True when a ClearContext with a preserved seed was applied and
    // the transcript now holds a synthetic message awaiting an LLM execution
    // turn (user_text was cleared). This keeps `drain_mode` false so run_loop
    // makes the execution call instead of going idle. Both preserved flavours
    // must continue running; only the blank sentinel (nothing preserved)
    // stops without an LLM turn.
    let mut handoff_pending = false;
    // Control commands (/act, /sandbox) short-circuit without an LLM turn. A
    // compound input (/sandbox review) switches then runs the rest. EXCEPTION:
    // /clear_context with a preserved seed falls through to run_loop.
    if let Some((cmd, rest)) = crate::control_cmd::split_control_prefix(&user_text) {
        crate::control_cmd::apply(session, &cmd, &mut on_event).await?;
        // ClearContext with a preserved seed falls through to run_loop so the
        // model sees the continuity context; blank sentinel path (nothing
        // preserved) stops as before.
        if matches!(cmd, crate::control_cmd::ControlCmd::ClearContext)
            && !crate::control_cmd::is_clear_context_handoff(
                session.handoff_plan.as_deref().unwrap_or(""),
            )
        {
            handoff_pending = true;
            match rest {
                // Compound (/clear_context review) with a preserved seed:
                // keep the request so it is recorded as a real user prompt and
                // executed alongside the seed marker message (not discarded).
                Some(rest) => user_text = rest,
                None => {
                    user_text.clear();
                    images.clear();
                }
            }
        } else if let Some(rest) = rest {
            // Compound (/sandbox review): switch done; fall through to recording
            // which resolves `$skill` tokens and records user_text as prompt.
            user_text = rest;
        } else {
            on_event(SessionEvent::Done);
            return Ok(());
        }
    }
    // F2: recover promoted-but-unrecorded inputs before entry_drain_mode polls.
    input_recovery::recover_orphaned_inputs(session).await;
    // Replay cancelled subagent tasks from a prior interrupted run BEFORE the
    // new input enters the loop: resume each child, backfill the parent
    // tool_result, flip to Completed. No-op for children (no `task` tool).
    // The TUI passes prompts directly (not via store Delivery), so when the
    // user typed new input, cancelled subagents are abandoned, not replayed.
    let has_new_input = !user_text.is_empty() || !images.is_empty();
    crate::resume::replay_cancelled_tasks(session, has_new_input).await;
    // Safety net: any `tool_use` id left dangling by a prior interrupted batch
    // is answered with a synthetic error tool_result, avoiding the provider's
    // "unanswered tool_call" HTTP 400. Idempotent; runs before recording input.
    crate::dangling_tools::reconcile_dangling_tool_uses(session).await;
    // Resolve inline `$skill` tokens from the raw user text (headless path —
    // the TUI resolves before calling run). Covers both compound commands
    // (`/sandbox $review do it`) and plain prompts (`$review do it`). After
    // stripping, text may be empty if only `$skill` tokens were provided.
    let prev_skill = session.skill_prompt_cloned();
    user_text = crate::skill_resolve::resolve_inline_skills(session, &user_text);
    // Consumption-time activation must also reach the store (queue/steer
    // drains persist inside record_compound; this is the direct-prompt
    // twin), so a resume after this turn replays the resolved skill.
    crate::skill_resolve::persist_active_skill(session, &prev_skill).await;
    // A non-empty prompt records a real user message. An empty prompt means
    // "drain mode": the web drain relies on admitted steers/queues being
    // claimed at turn boundaries to supply the actual user input (trigger
    // injection + pending-first priority: see drain::entry_drain_mode).
    let has_text = !user_text.trim().is_empty();
    let has_images = !images.is_empty();
    if has_text || has_images {
        let user = Message::user_with_images(new_id(), user_text, &images);
        session.record(user).await;
    }
    let drain_mode = entry_drain_mode(session, has_text, has_images, handoff_pending).await;
    // Zero-resubmit: a failed run must NOT re-submit admitted inputs.
    // Queue/steer rows stay pending (or are unpromoted in place by the
    // P1-3/F2 guards) and are consumed by the NEXT successful run —
    // a failed attempt never fires additional LLM requests for them.
    run_loop_one_shot(session, registry, &mut on_event, drain_mode).await?;

    // P1-4: bounded re-absorb of steers/queues admitted during run_loop's
    // idle window (see drain::reabsorb_tail).
    reabsorb_tail(session, registry, &mut on_event).await?;

    // Autopilot mode dispatch: after the initial task completes, `ap` hands
    // control to the PLAN -> ACT -> VERIFY self-driving loop, `review` runs a
    // one-shot review pass (no ACT/VERIFY), and `off` does nothing. A
    // session-scoped override (`effective_ap_mode`) wins over the config.
    // The review pass runs in ANY agent mode: it is read-only (no switch, no
    // fold), so it is equally valid after an act run or a sandbox run.
    match session.effective_ap_mode() {
        opencoder_core::ApMode::Ap => {
            crate::autopilot::drive(session, registry, &mut on_event).await?;
        }
        opencoder_core::ApMode::Review => {
            crate::autopilot::review_pass(session, registry, &mut on_event).await?;
        }
        opencoder_core::ApMode::Off => {}
    }
    Ok(())
}

pub(crate) async fn run_loop(
    session: &mut SessionState,
    registry: &HashMap<String, ToolArc>,
    on_event: &mut (dyn FnMut(SessionEvent) + Send),
    mut drain_mode: bool,
) -> Result<()> {
    let mut doom: VecDeque<String> = VecDeque::new();
    let mut tool_failures: crate::tool_guard::FailureMap = HashMap::new();
    // Tracks the first bash-timeout output in a consecutive run (paired with
    // the command's input) so subsequent timeouts for the SAME command can be
    // deduplicated (same PID / output file). Different commands start a new
    // streak so their distinct PIDs are preserved.
    let mut bash_timeout_first: Option<(String, Value)> = None;
    let mut skip_llm = false;
    // Consecutive drain-mode ConsumeNext steps without an intervening LLM
    // turn or steer absorption. A persistent claim failure mixed with
    // successful pending reads would otherwise hot-spin the loop; capping
    // the streak forces Done so the frontend resync can restart slowly.
    let mut consume_streak: u32 = 0;

    loop {
        // Interrupt check: if a cancellation was requested (web POST /interrupt),
        // stop cleanly at this turn boundary.
        if let Some(c) = &session.cancel {
            if c.is_cancelled() {
                on_event(SessionEvent::Status("interrupted".into()));
                // Terminal frame: without `Done` the SSE stream never closes
                // and the web console stays stuck in `streaming…` (busy) until
                // a manual reload — found by real-browser acceptance.
                on_event(SessionEvent::Done);
                break;
            }
        }
        // Capture+clear skip_llm from a previous bare-command drain.
        let skip = std::mem::replace(&mut skip_llm, false);

        // Safe Provider-Turn Boundary: promote any steers admitted since the
        // last turn. A steer is absorbed into history HERE.
        // Snapshot the child admission epoch before polling the store. A
        // commit racing this poll advances the epoch, so the final idle gate
        // cannot close until another boundary has observed the new input.
        let mut steer_epoch = session.steer_gate.as_ref().map(|gate| gate.epoch());
        let mut steer_recorded = false;
        let steer_prompts = claim_steers(session).await;
        // Admissions committed while the store poll was running are either
        // already in `steer_prompts` or remain durable pending rows that the
        // idle late-peek below will see. Advancing the observed epoch here
        // avoids an unnecessary empty provider turn when the poll did claim
        // the racing input.
        if let Some(gate) = &session.steer_gate {
            steer_epoch = Some(gate.epoch());
        }
        if !steer_prompts.is_empty() {
            // Steer absorption is loop progress — reset the drain consume
            // streak so the cap only counts back-to-back no-progress steps.
            consume_streak = 0;
            match apply_steer_batch(session, &mut *on_event, &steer_prompts).await? {
                SteerApplyOutcome::Continue { recorded } => steer_recorded = recorded,
                // Sentinel/bare-command-only batch, or hard cancel mid-batch
                // (Done / Status("interrupted") already emitted by the helper):
                // end this run — the frontend resync restarts as needed.
                SteerApplyOutcome::Done | SteerApplyOutcome::Cancelled => break,
            }
        }
        // Drain-mode pre-consume: process queue before the first LLM call
        // (web drain_to_completion). Bare commands loop via continue.
        if drain_mode && steer_prompts.is_empty() {
            match drain_mode_step(session, &mut *on_event, steer_epoch).await? {
                DrainModeAction::Proceed => {
                    drain_mode = false;
                    consume_streak = 0;
                }
                DrainModeAction::ConsumeNext => {
                    consume_streak += 1;
                    if consume_streak >= MAX_CONSUME_STREAK {
                        tracing::warn!(
                            streak = consume_streak,
                            "drain consume streak exceeded cap; going idle"
                        );
                        on_event(SessionEvent::Done);
                        break;
                    }
                    continue;
                }
                DrainModeAction::Idle => {
                    on_event(SessionEvent::Done);
                    break;
                }
            }
        }

        // Skip LLM: a bare control command was drained last idle boundary.
        if skip && !steer_recorded {
            match idle_drain(session, &mut *on_event, steer_epoch).await? {
                IdleAction::Continue => continue,
                IdleAction::SkipLlm => {
                    skip_llm = true;
                    continue;
                }
                IdleAction::Done => {
                    on_event(SessionEvent::Done);
                    break;
                }
            }
        }

        if compaction::should_compact(session) {
            // Retry compaction a few times (transient LLM failures like rate
            // limits are common) before giving up. On final failure return Err
            // so the caller decides what to do — falling through to
            // run_one_llm_call with an over-budget transcript would guarantee
            // a context-length 400 and kill the session.
            let mut last_err: Option<anyhow::Error> = None;
            for attempt in 0..=2u8 {
                match compaction::compact(session, registry, &mut *on_event).await {
                    Ok(Some(summary)) => {
                        on_event(SessionEvent::TranscriptReset(session.messages.clone()));
                        on_event(SessionEvent::Compaction(summary));
                        last_err = None;
                        break;
                    }
                    Ok(None) => {
                        // should_compact fired but there is nothing to
                        // summarize: an empty or single-message transcript.
                        // Two causes are possible: a stale reported usage
                        // from before a transcript collapse (clear-context /
                        // handoff now reset it), or a single message
                        // so large that the estimate alone crosses the
                        // compaction budget.
                        //
                        // The compaction budget is a threshold, not a hard
                        // cap: if the current transcript still fits under the
                        // provider context limit, proceed unchanged — killing
                        // the run here would strand a perfectly serviceable
                        // fresh-start turn. Only fail when the request is
                        // guaranteed to exceed the hard limit (nothing to
                        // summarize AND nothing left to ship).
                        if compaction::estimated_tokens(session) < session.config.context_limit() {
                            tracing::warn!(
                                "compaction found nothing to summarize; transcript fits under the hard context limit, proceeding uncompacted"
                            );
                            break;
                        }
                        last_err = Some(anyhow!(
                            "transcript exceeds context window but compaction found nothing to summarize"
                        ));
                        break;
                    }
                    Err(e) => {
                        last_err = Some(e);
                        if attempt < 2 {
                            on_event(SessionEvent::Status(format!(
                                "compaction retry {}/2",
                                attempt + 1
                            )));
                        }
                    }
                }
            }
            if let Some(e) = last_err {
                on_event(SessionEvent::Error(format!("compaction failed: {e:#}")));
                return Err(e);
            }
            // Compaction replaced the transcript with a fresh summary, so
            // stale doom-loop signatures, tool-failure counts, and
            // bash-timeout dedup streaks from pre-compaction turns must be
            // cleared to avoid false trips after compaction.
            doom.clear();
            tool_failures.clear();
            bash_timeout_first = None;
        }

        // Skill full-body injection: idempotent persistent `[skill loaded]`
        // message so the model never burns a tool call reading the SKILL.md.
        crate::skill_context::ensure_full_body_loaded(session).await;

        on_event(SessionEvent::LlmRoundStart {
            started_at_ms: now_ms(),
        });
        let turn = match run_one_llm_call(session, registry, on_event).await {
            Ok(t) => t,
            Err(e) => {
                on_event(SessionEvent::LlmRoundEnd);
                on_event(SessionEvent::Error(format!("{e:#}")));
                return Err(e);
            }
        };
        // Interrupt handling: turn-cancel (subagent steer) → reset + continue;
        // hard-cancel (web /stop) → break without persisting the empty turn
        // ("interrupted" status was already emitted by run_one_llm_call).
        if is_turn_cancelled(session) {
            on_event(SessionEvent::LlmRoundEnd);
            reset_turn_cancel(session);
            // A cancelled turn discards partial work the same way compaction
            // does: clear stale doom-loop signatures, tool-failure counts,
            // and bash-timeout dedup streaks so they don't false-trip after
            // the next steer resumes.
            doom.clear();
            tool_failures.clear();
            bash_timeout_first = None;
            continue;
        }
        if session.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            on_event(SessionEvent::LlmRoundEnd);
            // Status("interrupted") came from run_one_llm_call; still owe the
            // terminal `Done` frame — without it the SSE stream never closes
            // and the console stays busy forever (real-browser acceptance).
            on_event(SessionEvent::Done);
            break;
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
        if let Some(u) = &usage {
            on_event(SessionEvent::LlmUsage {
                total_tokens: u.total_tokens,
                input_tokens: u.input_tokens,
                output_tokens: u.output_tokens,
            });
        }

        if tool_calls.is_empty() {
            on_event(SessionEvent::LlmRoundEnd);
            // Late turn-cancel: capture an interrupt fired during
            // record().await above so it does not strand a queued input
            // via the biased select in idle_drain → claim_one_queued.
            if is_turn_cancelled(session) {
                reset_turn_cancel(session);
                // A cancelled turn discards partial work the same way
                // compaction does: clear stale doom-loop signatures,
                // tool-failure counts, and bash-timeout dedup streaks so
                // they don't false-trip after the next steer resumes.
                doom.clear();
                tool_failures.clear();
                bash_timeout_first = None;
                continue;
            }
            // Idle boundary: pop exactly one queued follow-up. Bare control
            // commands set skip_llm for the next iteration; a real prompt
            // continues the outer loop for an LLM turn.
            match idle_drain(session, &mut *on_event, steer_epoch).await? {
                IdleAction::Continue => continue,
                IdleAction::SkipLlm => {
                    skip_llm = true;
                    continue;
                }
                IdleAction::Done => {
                    on_event(SessionEvent::Done);
                    break;
                }
            }
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
                    emit(&sink, SessionEvent::LlmRoundEnd);
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
                    // A panic inside a tool's `execute` must not propagate out
                    // of FuturesUnordered and abort the whole run_loop (it
                    // would strand in-flight subagent futures and leave DB
                    // tasks in `Running`). Catch it and convert to an error
                    // ToolResult, matching how execute_call itself reports
                    // failures (is_error: true).
                    let out = match AssertUnwindSafe(execute_call(tc, session_ref, registry, &sink))
                        .catch_unwind()
                        .await
                    {
                        Ok(o) => o,
                        Err(payload) => ToolOutput::err(format!(
                            "tool `{}` panicked: {}",
                            tc.name,
                            panic_message(&payload)
                        )),
                    };
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
            dedup::dedup_consecutive_bash_timeouts(
                &tool_calls,
                &mut results,
                &mut bash_timeout_first,
            );
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
                        tokio::select! {
                            _ = tokio::time::sleep(max_delay) => {}
                            _ = await_cancel(session) => {}
                        }
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
            on_event(SessionEvent::LlmRoundEnd);
            on_event(SessionEvent::Status("interrupted".into()));
            // Terminal frame (same contract as the loop-head exit above).
            on_event(SessionEvent::Done);
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
        on_event(SessionEvent::LlmRoundEnd);

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
