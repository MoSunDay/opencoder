//! Queue consumption for drain mode: claiming/popping one queued input at a
//! time, the drain-mode pre-consume step, idle-boundary draining, the
//! ConsumeNext streak cap, and the bounded post-run_loop re-absorb tail.
//! Steer claiming/peeking itself lives in `steer.rs`; this module owns the
//! Queue side plus the shared drain stepping that ties both together.

use std::collections::HashMap;

use anyhow::Result;
use opencoder_core::{Message, Role, ToolArc};
use opencoder_store::Delivery;

use super::input_recovery::{mark_input_recorded, unpromote_batch};
use super::new_id;
use super::steer::{cancel_guard, has_pending_steers};
use crate::skill_lifecycle::run_loop_one_shot;
use crate::{SessionEvent, SessionState};

/// Cap on consecutive drain-mode `ConsumeNext` steps (bare commands / late
/// pending re-checks) before run_loop forces `Done`. Guards against a
/// hot-spin when queue claims keep failing while pending reads keep
/// succeeding; the frontend resync restarts the drain slowly and heals.
pub(super) const MAX_CONSUME_STREAK: u32 = 32;

/// Claim exactly one queued input at idle. Returns its (row seq, prompt), or
/// None. A persistently failing claim (initial attempt + single retry both
/// Err) emits a `SessionEvent::Error` on `on_event` so the stranded row is
/// visible on the event stream — the run still proceeds as if the queue were
/// empty (Empty semantics → Done), never failing the whole run.
pub(super) async fn claim_one_queued(
    session: &mut SessionState,
    on_event: &mut (dyn FnMut(SessionEvent) + Send),
) -> Option<(i64, String, Vec<String>)> {
    let store = session.store.clone()?;
    let sid = session.id.clone();
    // No cancel-guard select here. claim_next_queue runs
    // BEGIN IMMEDIATE -> SELECT -> UPDATE -> COMMIT atomically; racing the
    // hard cancel via a biased select could drop the future mid-COMMIT,
    // leaving the item permanently promoted (invisible to future queries)
    // yet never recorded as a user message -- permanent data loss. The whole
    // transaction completes in <1ms on local SQLite; the run loop's
    // top-of-loop interrupt check catches cancellation on the next iteration.
    match store.claim_next_queue(&sid).await {
        Ok(Some((seq, input))) => Some((seq, input.prompt, input.images.clone())),
        Ok(None) => None,
        Err(e) => {
            // Transient contention (e.g. a concurrent writer racing the
            // BEGIN IMMEDIATE transaction) can surface as Err here; treating
            // it as Empty strands the row pending forever while run_loop
            // reports Done. Retry exactly once — a persistent failure still
            // falls through to None, but is surfaced as an Error event so the
            // stranding is never silent (P2-4).
            tracing::warn!(error = %e, "claim_one_queued failed, retrying once");
            match store.claim_next_queue(&sid).await {
                Ok(Some((seq, input))) => Some((seq, input.prompt, input.images.clone())),
                Ok(None) => None,
                Err(e2) => {
                    tracing::warn!(error = %e2, "claim_one_queued retry failed");
                    on_event(SessionEvent::Error(format!(
                        "queued input claim failed: {e2:#}"
                    )));
                    None
                }
            }
        }
    }
}

/// Outcome of popping exactly one queued input at a turn/idle boundary.
#[derive(Debug)]
pub(super) enum DrainOutcome {
    /// A real prompt (or compound command rest, or ClearContext sentinel)
    /// was consumed and recorded. The caller should proceed to an LLM turn.
    Prompt,
    /// A bare control command was applied inline (agent switch etc.) with
    /// no real prompt. The caller should skip the LLM call and drain the
    /// next item on the following loop iteration.
    ControlCmd,
    /// The queue is empty — nothing was popped.
    Empty,
}

/// Pop exactly **one** queued input at an idle/turn boundary. Applies bare
/// control commands inline (no LLM turn), records real prompts via
/// [`crate::skill_resolve::record_compound`].
///
/// Unlike the previous `drain_queued` which looped internally and could pop
/// multiple items per call (bare commands via `continue`), this pops at most
/// one item. The caller re-invokes on the next loop iteration (skipping the
/// LLM call) to drain subsequent items, giving the outer loop a chance to
/// check for interrupts and new steers between each pop.
pub(super) async fn drain_one_queued(
    session: &mut SessionState,
    on_event: &mut (dyn FnMut(SessionEvent) + Send),
) -> Result<DrainOutcome> {
    if let Some((seq, q, imgs)) = claim_one_queued(session, on_event).await {
        // Hard-cancel guard between claim and apply: a cancel that fired
        // after the atomic claim must NOT apply a queued control command
        // (mode switch under a cancelled run). Unpromote the claimed row so
        // the next explicit run re-absorbs it and report Empty — the run
        // loop's top-of-loop check picks up the cancel and stops. The guard
        // sits BEFORE the QueueConsumed event: the TUI mirror drops the row
        // on that event while a cancelled run suppresses the Done resync,
        // which would leave the badge gone but the store row pending. The
        // claim itself stays deliberately unguarded (atomic
        // BEGIN IMMEDIATE..COMMIT; a biased-select race could strand the
        // row promoted-but-unclaimed — permanent data loss).
        if session.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            unpromote_batch(session, &[seq]).await;
            return Ok(DrainOutcome::Empty);
        }
        on_event(SessionEvent::QueueConsumed {
            seq,
            text: q.clone(),
        });
        if let Some((cmd, rest)) = crate::control_cmd::split_control_prefix(&q) {
            if let Err(e) = crate::control_cmd::apply(session, &cmd, &mut *on_event).await {
                // P1-3: unpromote the claimed item so the next run retries it.
                if let Some(store) = &session.store {
                    let _ = store
                        .unpromote_inputs(&session.id, std::slice::from_ref(&seq))
                        .await;
                }
                return Err(e);
            }
            // Compound (/plan review, /act_clear_context review): rest is a
            // real prompt in the new mode — record it and break.
            if let Some(rest) = rest {
                crate::skill_resolve::record_compound(session, &rest, &imgs).await;
                mark_input_recorded(session, seq).await;
                return Ok(DrainOutcome::Prompt);
            }
            // Bare ClearContext with a preserved seed falls through so the
            // model sees the continuity context; blank sentinel (nothing
            // preserved) goes idle.
            if matches!(cmd, crate::control_cmd::ControlCmd::ClearContext)
                && !crate::control_cmd::is_clear_context_handoff(
                    session.handoff_plan.as_deref().unwrap_or(""),
                )
            {
                mark_input_recorded(session, seq).await;
                return Ok(DrainOutcome::Prompt);
            }
            // Bare command: applied, no LLM turn needed.
            mark_input_recorded(session, seq).await;
            return Ok(DrainOutcome::ControlCmd);
        }
        // Real prompt: resolve `$skill` tokens, record, break.
        // F2: per-item marking (mirrors the steer loop) — never lost on failure.
        crate::skill_resolve::record_compound(session, &q, &imgs).await;
        mark_input_recorded(session, seq).await;
        return Ok(DrainOutcome::Prompt);
    }
    // Queue empty.
    Ok(DrainOutcome::Empty)
}

/// Action the caller should take after an idle-boundary drain.
#[derive(Debug)]
pub(super) enum IdleAction {
    /// A prompt (or late steer/queue) was found — continue the outer loop.
    Continue,
    /// A bare control command was applied — skip the next LLM call and
    /// drain again.
    SkipLlm,
    /// Queue empty and no late steer/queue — emit Done and stop.
    Done,
}

/// Drain one queued item at an idle boundary and determine the next action.
/// Encapsulates pop-one + late-steer/queue peek so [`run_loop`] can call it
/// from both the normal idle path and the skip-LLM path without duplication.
pub(super) async fn idle_drain(
    session: &mut SessionState,
    on_event: &mut (dyn FnMut(SessionEvent) + Send),
    steer_epoch: Option<u64>,
) -> Result<IdleAction> {
    match drain_one_queued(session, on_event).await? {
        DrainOutcome::Prompt => Ok(IdleAction::Continue),
        DrainOutcome::ControlCmd => Ok(IdleAction::SkipLlm),
        DrainOutcome::Empty => {
            let late_steer = has_pending_steers(session).await;
            if late_steer {
                // A steer admitted after our pop is claimed at the top of the
                // next loop iteration -- nothing to consume here.
                return Ok(IdleAction::Continue);
            }
            // A queued input may have been admitted in the gap between
            // claim_next_queue's SELECT and this peek. Consume it for real
            // instead of merely returning Continue: a bare Continue re-enters
            // run_loop whose top-of-loop claim_steers never checks the queue,
            // and a spurious LLM call could strand the item during thinking.
            if has_pending_queues(session).await {
                match drain_one_queued(session, on_event).await? {
                    DrainOutcome::Prompt => return Ok(IdleAction::Continue),
                    DrainOutcome::ControlCmd => return Ok(IdleAction::SkipLlm),
                    DrainOutcome::Empty => {} // vanished between peek and pop
                }
            }
            if let (Some(gate), Some(epoch)) = (&session.steer_gate, steer_epoch) {
                match gate.settle_idle(epoch).await {
                    crate::subagent_steer_gate::IdleDecision::Continue => Ok(IdleAction::Continue),
                    crate::subagent_steer_gate::IdleDecision::Close => Ok(IdleAction::Done),
                }
            } else {
                Ok(IdleAction::Done)
            }
        }
    }
}

/// Action for `run_loop` after a drain-mode pre-consume step.
pub(super) enum DrainModeAction {
    /// A real prompt was consumed (or transcript needs a response) — proceed
    /// to the LLM call.
    Proceed,
    /// A bare command was applied, or a late steer/queue appeared — loop back.
    ConsumeNext,
    /// Queue empty, nothing pending — go idle.
    Idle,
}

/// One step of drain-mode pre-consume: pop a queued input and decide whether
/// to proceed to the LLM call, loop back for the next item, or go idle.
/// Called only when `drain_mode` is active and no steers are pending.
pub(super) async fn drain_mode_step(
    session: &mut SessionState,
    on_event: &mut (dyn FnMut(SessionEvent) + Send),
    steer_epoch: Option<u64>,
) -> Result<DrainModeAction> {
    match drain_one_queued(session, on_event).await? {
        DrainOutcome::Prompt => Ok(DrainModeAction::Proceed),
        DrainOutcome::ControlCmd => Ok(DrainModeAction::ConsumeNext),
        DrainOutcome::Empty => {
            // Queue empty. If the transcript ends with an unresponded user
            // message (e.g. an execution handoff awaiting run), proceed
            // to the LLM call. A trailing Role::Tool message is equally
            // "unresponded" — the model must process the tool result — so
            // treat it the same way. Without this, a drain-mode session that
            // ran a tool call (driven by a steer) would go Idle with the
            // tool result stranded and never answered.
            // Exclude the clear-context fresh-start sentinel.
            let last_role = session.messages.last().map(|m| m.role);
            let needs_llm = match last_role {
                Some(Role::Tool) => true,
                Some(Role::User) => !session
                    .handoff_plan
                    .as_deref()
                    .is_some_and(crate::control_cmd::is_clear_context_handoff),
                _ => false,
            };
            // Late-check FIRST: a steer/queue admitted after the pop must be
            // consumed before we proceed to the LLM, otherwise the item is
            // stranded for the duration of the thinking phase.
            if has_pending_steers(session).await || has_pending_queues(session).await {
                return Ok(DrainModeAction::ConsumeNext);
            }
            if needs_llm {
                return Ok(DrainModeAction::Proceed);
            }
            if let (Some(gate), Some(epoch)) = (&session.steer_gate, steer_epoch) {
                match gate.settle_idle(epoch).await {
                    crate::subagent_steer_gate::IdleDecision::Continue => {
                        Ok(DrainModeAction::ConsumeNext)
                    }
                    crate::subagent_steer_gate::IdleDecision::Close => Ok(DrainModeAction::Idle),
                }
            } else {
                Ok(DrainModeAction::Idle)
            }
        }
    }
}

/// Peek (read-only) whether any Queue inputs are pending for this session,
/// WITHOUT claiming them. Used at the idle boundary (text-only turn, empty
/// queue) to close the race where a queued input is admitted after
/// `claim_one_queued` returns None but before `Done` would strand it.
/// Symmetric with [`has_pending_steers`]. Returns false when no store is
/// attached or the read fails (fail-open: go idle).
pub(super) async fn has_pending_queues(session: &SessionState) -> bool {
    let Some(store) = session.store.clone() else {
        return false;
    };
    let sid = session.id.clone();
    let hard = session.cancel.clone();
    tokio::select! {
        biased;
        _ = cancel_guard(hard) => false,
        v = async {
            match store.pending_inputs(&sid, Delivery::Queue).await {
                Ok(v) => !v.is_empty(),
                Err(e) => {
                    tracing::warn!(error = %e, "has_pending_queues: pending_inputs failed");
                    false
                }
            }
        } => v,
    }
}

/// P1-4: Bounded re-absorb — if a steer/queue was admitted during the idle
/// window (between run_loop's last pending_inputs poll and its return),
/// re-run with drain_mode to absorb it. Without this, the TUI (which has
/// no web-style reaper) would strand the input until the next manual
/// submit. Max 3 re-checks to bound latency. Checks BOTH steers and queued
/// inputs: a queue follow-up admitted during the idle window would otherwise
/// be stranded exactly like a bare steer.
pub(super) async fn reabsorb_tail(
    session: &mut SessionState,
    registry: &HashMap<String, ToolArc>,
    on_event: &mut (dyn FnMut(SessionEvent) + Send),
) -> Result<()> {
    let mut rechecks = 0u32;
    const MAX_RECHECKS: u32 = 3;
    while rechecks < MAX_RECHECKS
        && (has_pending_steers(session).await || has_pending_queues(session).await)
    {
        rechecks += 1;
        run_loop_one_shot(session, registry, on_event, true).await?;
    }
    Ok(())
}

/// Entry-point drain decision + active-skill trigger injection.
///
/// When an active skill is set and the user submitted no text (pure-skill
/// submit after token stripping or image-only), inject a synthetic trigger so
/// the model records a user turn and acts on the skill body in the system
/// prompt instead of treating the input passively. For text-bearing turns the
/// user's own words drive execution. EXCEPTION: when steers/queues are
/// already pending, the pending input wins (FIFO) — drain mode pops the queue
/// instead of re-triggering the active skill. Without this, a drain restart
/// (TUI drain_pending / web drain_to_completion) injected a fresh
/// SKILL_TRIGGER every cycle and never popped the queue: a self-continuing
/// loop that repeatedly re-activated the same skill.
///
/// Under one-shot skill semantics (see `skill_lifecycle`) a normally
/// completed run has already cleared the skill at its end, so this trigger
/// path only fires for the triggering round itself (the same run that
/// activated the skill) or for a resumed / pre-set skill_prompt (mid-run
/// crash recovery, web SetSkill-before-run) — never for a stale skill left
/// over from an earlier, finished run.
///
/// Pending rows are polled ONLY on a pure-drain submit that still has an
/// active skill (the `!has_skill` disjunct short-circuits the reads): a
/// text/image-bearing turn never consults the store, so its poll sequence —
/// and the P1-4 deterministic re-absorb window — stays untouched.
pub(super) async fn entry_drain_mode(
    session: &mut SessionState,
    has_text: bool,
    has_images: bool,
    handoff_pending: bool,
) -> bool {
    let has_skill = session.skill_prompt_cloned().is_some();
    let drain_mode = !has_text
        && !has_images
        && !handoff_pending
        && (!has_skill || has_pending_steers(session).await || has_pending_queues(session).await);
    if has_skill && !has_text && !drain_mode {
        let mut msg = Message::user(new_id(), crate::skill_resolve::SKILL_TRIGGER);
        msg.synthetic = true;
        session.record(msg).await;
    }
    drain_mode
}

#[cfg(test)]
#[path = "drain_tests.rs"]
mod drain_tests;
