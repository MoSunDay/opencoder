use super::*;

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
/// the headless CLI (`opencoder run "..." --image ./a.png`).
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
    user_text: String,
    images: Vec<String>,
    registry: &HashMap<String, ToolArc>,
    on_event: impl FnMut(SessionEvent) + Send,
) -> Result<()> {
    let root = session.config.agent.agents_dir.clone();
    opencoder_core::agent::scope::with_root(
        root,
        run_with_registry_scoped(session, user_text, images, registry, on_event),
    )
    .await
}

async fn run_with_registry_scoped(
    session: &mut SessionState,
    mut user_text: String,
    mut images: Vec<String>,
    registry: &HashMap<String, ToolArc>,
    on_event: impl FnMut(SessionEvent) + Send,
) -> Result<()> {
    let mut on_event = on_event;
    let _loop_guard = crate::loop_registry::LoopGuard::enter(&session.id);
    // True when ClearContext produced a synthetic input awaiting an LLM turn:
    // a neutral act-mode seed or a plan→act execution directive. This keeps
    // `drain_mode` false so run_loop executes instead of going idle. Only the
    // blank sentinel (nothing preserved) stops without an LLM turn.
    let mut handoff_pending = false;
    // Control commands (/act, /plan) short-circuit without an LLM turn. A
    // compound input (/plan review) switches then runs the rest. EXCEPTION:
    // /act_clear_context with a seed or plan directive falls through to
    // run_loop.
    if let Some((cmd, rest)) = crate::control_cmd::split_control_prefix(&user_text) {
        if let Err(e) = crate::control_cmd::apply(session, &cmd, &mut on_event).await {
            // This path returns before the one-shot wrapper; honor the
            // run-end skill contract before propagating. Non-task-plan
            // skills must not survive a failed control-command run;
            // task-plan does (aborted plan = never delivered).
            if !crate::skill_lifecycle::abort_keeps_skill(session, true) {
                crate::skill_lifecycle::clear_on_run_end(session, &mut on_event).await;
            }
            return Err(e);
        }
        // ClearContext with a preserved seed/directive falls through to
        // run_loop; blank sentinel path (nothing preserved) stops as before.
        if matches!(cmd, crate::control_cmd::ControlCmd::ClearContext)
            && !crate::control_cmd::is_clear_context_handoff(
                session.handoff_plan.as_deref().unwrap_or(""),
            )
        {
            handoff_pending = true;
            match rest {
                // Compound (/act_clear_context review) with a preserved seed:
                // keep the request so it is recorded as a real user prompt and
                // executed alongside the seed marker message (not discarded).
                Some(rest) => user_text = rest,
                None => {
                    user_text.clear();
                    images.clear();
                }
            }
        } else if let Some(rest) = rest {
            // Compound (/plan review): switch done; fall through to recording
            // which resolves `$skill` tokens and records user_text as prompt.
            user_text = rest;
        } else {
            // Bare control command: this "run" returns before the one-shot
            // wrapper, so clear the skill here too — otherwise a
            // crash-resume-armed skill survives a bare `/act` //`/sandbox`
            // and resurrects into every later run.
            crate::skill_lifecycle::clear_on_run_end(session, &mut on_event).await;
            on_event(SessionEvent::Done);
            return Ok(());
        }
    }
    crate::harness::resources::prepare(session)?;
    // F2: recover promoted-but-unrecorded inputs before entry_drain_mode polls.
    input_recovery::recover_orphaned_inputs(session).await;
    // Replay cancelled subagent tasks from a prior interrupted run BEFORE the
    // new input enters the loop: resume each child, backfill the parent
    // tool_result, flip to Completed. No-op for children (no `task` tool).
    // The TUI passes prompts directly (not via store Delivery), so when the
    // user typed new input, cancelled subagents are abandoned, not replayed.
    let has_new_input = !user_text.is_empty() || !images.is_empty();
    if session.harness.harness == opencoder_core::harness::Harness::Opencoder {
        crate::resume::replay_cancelled_tasks(session, has_new_input).await;
    }
    // Safety net: any `tool_use` id left dangling by a prior interrupted batch
    // is answered with a synthetic error tool_result, avoiding the provider's
    // "unanswered tool_call" HTTP 400. Idempotent; runs before recording input.
    crate::dangling_tools::reconcile_dangling_tool_uses(session).await;
    // Resolve inline `$skill` tokens from the raw user text (headless path —
    // the TUI resolves before calling run). Covers both compound commands
    // (`/plan $review do it`) and plain prompts (`$review do it`). After
    // stripping, text may be empty if only `$skill` tokens were provided.
    let prev_skill = session.skill_prompt_cloned();
    // Capture the verbatim input BEFORE resolution: `display` records the
    // raw prompt (`$skill` tokens included) for every echo surface, while
    // the resolved clean text below is what the LLM consumes.
    let raw_user_text = user_text.clone();
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
    // Verbatim echo text: the raw prompt as typed (may be emptied by skill
    // resolution — then it rides the entry trigger instead of a record).
    let verbatim = (!raw_user_text.trim().is_empty()).then(|| raw_user_text.clone());
    if has_text || has_images {
        // has_text => verbatim is Some (raw non-empty); images-only => raw
        // was empty, so verbatim is None and the record stays fallback-clean.
        let user = Message::user_with_display(new_id(), user_text, verbatim.clone(), &images);
        if session.harness.harness == opencoder_core::harness::Harness::Codex {
            session.record_checked(user).await?;
        } else {
            session.record(user).await;
        }
    }
    // A pure-skill submit (tokens stripped to empty) surfaces its trigger
    // through entry_drain_mode; hand the verbatim input along so the
    // injected trigger replays as the user's own words.
    let drain_mode =
        entry_drain_mode(session, has_text, has_images, handoff_pending, verbatim).await;
    // Zero-resubmit: a failed run must NOT re-submit admitted inputs.
    // Queue/steer rows stay pending (or are unpromoted in place by the
    // P1-3/F2 guards) and are consumed by the NEXT successful run —
    // a failed attempt never fires additional LLM requests for them.
    run_loop_one_shot(session, registry, &mut on_event, drain_mode).await?;

    // P1-4: bounded re-absorb of steers/queues admitted during run_loop's
    // idle window (see drain::reabsorb_tail).
    reabsorb_tail(session, registry, &mut on_event).await?;
    if session.harness.harness == opencoder_core::harness::Harness::Codex {
        return Ok(());
    }

    // Autopilot mode dispatch: after the initial task completes, `ap` hands
    // control to the PLAN -> ACT -> VERIFY self-driving loop, `review` runs a
    // one-shot review pass (no ACT/VERIFY), and `off` does nothing. A
    // session-scoped override (`effective_ap_mode`) wins over the config.
    // The review pass runs in ANY agent mode: it is read-only (no switch, no
    // fold), so it is equally valid after an act run or a plan run.
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
