//! Session recovery: reconstruct a `SessionState` from a durable store, and
//! cheap background title generation (uses `small_model`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use opencoder_core::{
    message::now_ms, resolve_agent, Config, ContentBlock, Message, MessageUsage, Role,
};
use opencoder_llm::{lower_messages, ChatRequest, ChatStream, LlmEvent};
use opencoder_store::{
    Delivery, EventKind, SessionEventRecord, Store, SubagentStatus, SubagentTaskRecord,
};

use crate::SessionState;
use tokio_util::sync::CancellationToken;

/// Rebuild a session from persisted history. The agent/model come from the
/// stored session metadata when available, so a resumed session keeps its
/// original configuration rather than the caller's defaults.
pub async fn resume(
    store: Arc<dyn Store>,
    id: &str,
    mut config: Config,
    client: Arc<dyn ChatStream>,
    working_dir: PathBuf,
) -> Result<SessionState> {
    let meta = store
        .get_session(id)
        .await?
        .ok_or_else(|| anyhow!("session not found: {id}"))?;

    // Prefer the stored model/agent so resume is faithful to the original run.
    if let Some(m) = &meta.model {
        config.model = m.clone();
    }
    // Session-scoped autopilot mode (`/ap` session-only): the override wins
    // over the global config at the runner's dispatch point. NULL follows the
    // global config; unknown values warn and are ignored.
    let ap_mode_override = meta.autopilot_mode.as_deref().and_then(|v| {
        opencoder_core::ApMode::parse(v).or_else(|| {
            tracing::warn!(session_id = %id, mode = %v, "unknown sessions.autopilot_mode; ignoring");
            None
        })
    });
    let agent_name = meta.agent.as_deref().unwrap_or(&config.agent.default);
    let agent = resolve_agent(agent_name)
        .or_else(|| resolve_agent("act"))
        .ok_or_else(|| anyhow!("agent not found: {agent_name}"))?;

    // Loading strategy:
    //  - Compaction path (summary_seq set, no handoff): load ONLY the tail
    //    after the compacted head via OFFSET. The head's surviving images are
    //    now persisted (summary_images), so the head never needs to be loaded
    //    to re-derive them -- the fix for long-session resume stalls caused by
    //    reloading + deserializing thousands of soft-deleted head messages.
    //  - Handoff / no-compaction path: full load. Handoff is an early one-time
    //    handoff transition with small data; no-compaction has nothing to skip.
    let mut messages: Vec<Message> =
        if meta.handoff_seq.is_none() && matches!(meta.summary_seq, Some(sk) if sk > 0) {
            store
                .load_messages_after(id, meta.summary_seq.unwrap())
                .await?
        } else {
            store.load_messages(id).await?
        };

    // Reconcile subagent tasks stuck in Running state — the process was
    // interrupted mid-subagent. Mark them Cancelled (not Failed): a cancelled
    // task keeps its parent tool_use open so it is replayed on the next user
    // turn (run_with_registry), rather than recording a terminal error result.
    let tasks = store.list_subagent_tasks(id).await.unwrap_or_default();
    for task in &tasks {
        if task.status == SubagentStatus::Running {
            tracing::warn!(task_id = %task.task_id, "marking stuck Running subagent as Cancelled on resume");
            let _ = store.cancel_subagent_task(&task.task_id).await;
        }
    }

    // Transcript handoff (dominant reset) and compaction are mutually exclusive
    // on resume: when a handoff boundary was persisted, trim the discarded
    // history and re-attach the synthetic boundary message; otherwise apply a
    // persisted compaction trim. Handoff wins because it replaces the whole
    // transcript, so any stale compaction metadata from the cleared history is moot.
    if let Some(hs) = meta.handoff_seq {
        if let Some(boundary_display) = &meta.handoff_plan {
            let hs = hs as usize;
            // The discarded head is still in the store; re-derive its
            // recent images and attach them to the handoff instruction so they
            // survive resume.
            let preserved_images = if hs < messages.len() {
                crate::compaction::collect_head_images(&messages[..hs])
            } else {
                Vec::new()
            };
            if hs < messages.len() {
                messages = messages[hs..].to_vec();
            } else {
                messages = Vec::new();
            }
            // Distinguish the ClearContext boundary flavours from a directive
            // handoff: the blank sentinel / last-say seed markers stored by
            // control_cmd::ClearContext.
            let mut head_msg = if crate::control_cmd::is_clear_context_handoff(boundary_display) {
                crate::control_cmd::fresh_start_message()
            } else if crate::control_cmd::is_clear_context_seed(boundary_display) {
                crate::control_cmd::seed_message(crate::control_cmd::clear_seed_text(
                    boundary_display,
                ))
            } else {
                crate::handoff::handoff_message(boundary_display)
            };
            for url in &preserved_images {
                head_msg.blocks.push(ContentBlock::Image {
                    url: url.clone(),
                    detail: None,
                });
            }
            messages.insert(0, head_msg);
        }
    } else if meta.summary_seq.is_some() {
        // Compaction path: `messages` already holds ONLY the post-compaction tail
        // (loaded via OFFSET above). Reconstruct the synthetic summary from the
        // persisted summary text + persisted summary_images, avoiding the old
        // collect_head_images call that forced a full head reload just to extract
        // a few image URLs.
        if let Some(summary_text) = &meta.summary {
            let mut summary_msg = crate::compaction::compaction_message(summary_text.clone());
            for url in &meta.summary_images {
                summary_msg.blocks.push(ContentBlock::Image {
                    url: url.clone(),
                    detail: None,
                });
            }
            messages.insert(0, summary_msg);
        }
    }

    // Reconcile dangling tool_use blocks. If the process was hard-interrupted
    // after the assistant requested tool calls but before the matching
    // tool_result messages were persisted, the transcript holds unmatched
    // `tool_use` ids -- which most OpenAI-compatible providers reject with
    // HTTP 400 on the next call. Synthesize error results for every dangling
    // call, persist them, and append them so history stays well-formed.
    // Pure logic lives in `dangling_tools` (shared with the in-process
    // safety net in `run_with_registry`); only the persistence differs.
    let replayable = crate::dangling_tools::replayable_task_ids_from_records(&tasks);
    let dangling = crate::dangling_tools::dangling_tool_use_results(&messages, &replayable);
    if !dangling.is_empty() {
        let n_dangling = dangling.len();
        let synthetic = Message {
            id: crate::runner::new_id(),
            role: Role::Tool,
            blocks: dangling,
            model: None,
            agent: None,
            usage: opencoder_core::MessageUsage::default(),
            created_at: opencoder_core::message::now_ms(),
            synthetic: true,
        };
        tracing::warn!(
            session_id = id,
            count = n_dangling,
            "synthesizing error tool_result for dangling tool_use on resume"
        );
        // Persist so a subsequent resume sees a well-formed transcript.
        let _ = store.append_message(id, &synthetic).await;
        messages.push(synthetic);
    }

    let n = messages.len();
    let model = config.model_id().to_string();

    // Handoff supersedes compaction: if a handoff boundary exists, any
    // residual compaction metadata (summary_seq / summary / summary_images)
    // left in the store is stale. The handoff persistence path now clears it
    // (clear_summary: true), but sessions created before that fix -- or any
    // path that sets handoff without the clear -- may still carry the residue.
    // Zero it out here so compaction's `prev_skip = summary_seq.or(handoff_seq)`
    // picks the correct handoff_seq, not a stale smaller summary_seq.
    let (summary, summary_seq, summary_images) = if meta.handoff_seq.is_some() {
        (None, None, Vec::new())
    } else {
        (
            meta.summary.clone(),
            meta.summary_seq,
            meta.summary_images.clone(),
        )
    };

    let s = SessionState {
        id: id.to_string(),
        messages,
        agent,
        model,
        ap_mode_override,
        working_dir,
        config,
        client,
        last_usage: opencoder_llm::Usage::default(),
        store: Some(store),
        // Restore the persisted skill. Under one-shot semantics a normally
        // COMPLETED run has already cleared `sessions.skill` (NULL row ->
        // nothing resurrects); a non-NULL value means the session crashed
        // MID-run, and the resumed run must continue the skill —
        // `skill_lifecycle::clear_on_run_end` clears it when that run ends
        // (sole exception: `abort_keeps_skill` keeps an aborted task-plan,
        // so an interrupted plan survives to be delivered after resume).
        skill_prompt: Arc::new(Mutex::new(meta.skill.clone())),
        active_skill_names: Arc::new(Mutex::new(crate::resume_helpers::infer_skill_names(
            &meta.skill,
        ))),
        persisted_count: n,
        session_created: true,
        ts_origin: false,
        cancel: None,
        turn_cancel: Some(Arc::new(Mutex::new(CancellationToken::new()))),
        child_turn_cancels: Arc::new(Mutex::new(HashMap::new())),
        child_steer_gates: Arc::new(Mutex::new(HashMap::new())),
        steer_gate: None,
        child_cancels: Arc::new(Mutex::new(HashMap::new())),
        summary,
        summary_seq,
        summary_images,
        handoff_seq: meta.handoff_seq,
        handoff_plan: meta.handoff_plan.clone(),
        requirement: meta.requirement.clone(),
        question_hub: crate::QuestionHub::new(),
    };
    Ok(s)
}

/// Replay subagent tasks stuck in `Running` for `id`, then resume the parent.
///
/// When a parent session is hard-interrupted mid-subagent, the task row is
/// left `Running` and the parent's transcript holds an unanswered `task`
/// `tool_use`. This resumes each such child from its persisted transcript,
/// runs it to completion with an empty prompt ("continue"), backfills the
/// resulting `tool_result` into the parent, and marks the task complete.
///
/// Children hold no `task` tool (see `agent.rs`), so a child can never
/// dispatch a grandchild — there is exactly one level and no recursion is
/// needed. The low-level [`resume`] is left untouched: by the time it runs,
/// no task is `Running` and every `task` `tool_use` is answered, so its
/// stuck-task and dangling-`tool_use` reconciliation paths are inert.
pub async fn resume_and_replay(
    store: Arc<dyn Store>,
    id: &str,
    config: Config,
    client: Arc<dyn ChatStream>,
    working_dir: PathBuf,
    replay_cancel: Option<CancellationToken>,
) -> Result<SessionState> {
    let candidates: Vec<SubagentTaskRecord> = store
        .list_subagent_tasks(id)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|t| {
            matches!(
                t.status,
                SubagentStatus::Running | SubagentStatus::Cancelled
            )
        })
        .collect();

    // Duplicate/orphan tool_result guard: replay ONLY tasks whose `tool_use`
    // is still unanswered AND visible above any handoff/compaction boundary
    // (see `filter_replay_candidates`). Unfiltered, a task whose result is
    // already persisted (timeout path recorded it but a crash left the row
    // non-terminal; or a prior `resume_and_replay` backfilled then crashed)
    // would get a DUPLICATE tool_result, and a task dispatched below a
    // boundary would get an ORPHAN result — both are provider HTTP-400
    // rejects that permanently break the session.
    let pending = filter_replay_candidates(&store, id, candidates).await;

    // Replay each non-terminal child (Running or Cancelled), collecting results to backfill in ONE Tool
    // message -- mirrors run_loop, which batches a turn's tool results into a
    // single tool message. `list_subagent_tasks` returns rows in `seq` order,
    // so results land deterministically in dispatch order.
    let mut backfill: Vec<ContentBlock> = Vec::with_capacity(pending.len());
    for task in &pending {
        if let Some(c) = &replay_cancel {
            if c.is_cancelled() {
                break;
            }
        }
        let outcome = replay_child(
            store.clone(),
            task,
            &config,
            &client,
            &working_dir,
            replay_cancel.as_ref(),
        )
        .await;
        let (text, ok) = match outcome {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    task_id = %task.task_id,
                    child = %task.child_session_id,
                    error = %e,
                    "subagent replay failed; backfilling an error result"
                );
                (format!("subagent resume failed: {e:#}"), false)
            }
        };
        let _ = store.complete_subagent_task(&task.task_id, &text, ok).await;
        backfill.push(ContentBlock::ToolResult {
            tool_use_id: task.task_id.clone(),
            content: text,
            is_error: !ok,
            images: Vec::new(),
        });
    }

    // Backfill the tool_results BEFORE resuming, so resume() sees every task
    // `tool_use` as answered and does not synthesize error results for them
    // via its dangling-`tool_use` reconciliation.
    if !backfill.is_empty() {
        let tool_msg = Message {
            id: crate::runner::new_id(),
            role: Role::Tool,
            blocks: backfill,
            model: None,
            agent: None,
            usage: MessageUsage::default(),
            created_at: now_ms(),
            synthetic: false,
        };
        if let Err(e) = store.append_message(id, &tool_msg).await {
            tracing::warn!(
                session_id = id,
                error = %e,
                "failed to backfill replayed tool_results; falling back to plain resume"
            );
        }
    }

    // All tasks are now complete and the task `tool_use` ids are answered, so
    // resume() reconstructs the parent cleanly.
    resume(store, id, config, client, working_dir).await
}

/// Replay-candidate guard for [`resume_and_replay`]: drop tasks whose replay
/// would corrupt the parent transcript —
///
/// (a) duplicate: the task's `tool_use` id already carries a `tool_result`
///     among the messages `resume` will show the model (timeout path recorded
///     the result but a crash left the row non-terminal, or an earlier
///     `resume_and_replay` backfilled and crashed before completion);
/// (b) orphan: a handoff/compaction boundary trimmed the dispatching
///     assistant message out of the visible tail, so a backfilled result
///     would answer a `tool_use` the provider never sees.
///
/// Both defects are provider HTTP-400 rejects. This mirrors the in-process
/// guard `replay_cancelled_tasks` applies, extended with the boundary check.
/// Dropped rows are left untouched (same semantics as that guard): a
/// `Running` row is reconciled to `Cancelled` by `resume` itself, and any
/// later `resume_and_replay` re-collects and re-filters it the same way —
/// the guard is idempotent, so a skip can never resurrect a duplicate.
async fn filter_replay_candidates(
    store: &Arc<dyn Store>,
    id: &str,
    candidates: Vec<SubagentTaskRecord>,
) -> Vec<SubagentTaskRecord> {
    if candidates.is_empty() {
        return candidates;
    }
    let visible = match store.get_session(id).await {
        Ok(Some(meta)) => crate::dangling_tools::visible_parent_tail(store, id, &meta).await,
        // Missing/unreadable meta: `resume` below surfaces the real error;
        // keep the pre-guard behavior rather than silently dropping tasks.
        _ => return candidates,
    };
    let answered = crate::dangling_tools::tool_result_ids(&visible);
    candidates
        .into_iter()
        .filter(|t| {
            let duplicate = answered.contains(t.task_id.as_str());
            let dispatch_visible = crate::dangling_tools::task_tool_use_visible(t, &visible);
            if duplicate || !dispatch_visible {
                tracing::info!(
                    task_id = %t.task_id,
                    duplicate_result = duplicate,
                    dispatch_visible,
                    "skipping subagent replay (duplicate/orphan tool_result guard)"
                );
            }
            !duplicate && dispatch_visible
        })
        .collect()
}

/// Replay subagent tasks left in `Cancelled` status, then mark them complete.
///
/// Called from `run_with_registry` before the main loop runs, so a continued
/// session resumes each cancelled child (run to completion), backfills the
/// resulting `tool_result` into the parent transcript, and flips the task to
/// `Completed`. The model then sees [user input + subagent result] together and
/// the interrupted call is transparently resumed. No-op when there is no store
/// or no cancelled tasks (e.g. children, which hold no `task` tool).
pub async fn replay_cancelled_tasks(session: &mut SessionState, has_new_input: bool) {
    let store = match session.store.clone() {
        Some(s) => s,
        None => return,
    };
    // tool_use_ids that already have a matching tool_result in the transcript.
    // A Cancelled task whose result is already present (e.g. a timed-out
    // subagent, whose parent recorded the "timed out" tool_result) must NOT be
    // replayed — doing so would append a duplicate tool_result that providers
    // reject with HTTP 400. Shared computation (also used by
    // `resume_and_replay`'s cross-process guard and the dangling-use safety
    // net): `dangling_tools::tool_result_ids`.
    let answered = crate::dangling_tools::tool_result_ids(&session.messages);
    let cancelled: Vec<SubagentTaskRecord> = store
        .list_subagent_tasks(&session.id)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|t| {
            t.status == SubagentStatus::Cancelled
                && !answered.contains(t.task_id.as_str())
                && (session.handoff_seq.is_none()
                    || session.messages.iter().any(|m| {
                        m.blocks.iter().any(
                            |b| matches!(b, ContentBlock::ToolUse { id, .. } if id == &t.task_id),
                        )
                    }))
        })
        .collect();
    if cancelled.is_empty() {
        return;
    }
    // Abandon (don't replay) the cancelled subagents when the user is moving
    // on to new input. Three signals trigger this:
    //  1. has_new_input — the TUI submits user_text directly (not via the store
    //     queue), so we pass a flag from run_loop instead of querying rows.
    //  2. pending steers — the web layer admits to the store first, then drains;
    //     a steer means the user explicitly redirected mid-subagent.
    //  3. pending queue — a queued prompt is waiting to be claimed.
    // In all three cases the user wants to move on, not silently resume the
    // interrupted child. Backfill a terminal "cancelled" tool_result so the
    // transcript stays well-formed, and mark each task Failed so it is never
    // replayed again.
    let has_pending_steers = store
        .pending_inputs(&session.id, Delivery::Steer)
        .await
        .map(|v| !v.is_empty())
        .unwrap_or(false);
    let has_pending_queue = store
        .pending_inputs(&session.id, Delivery::Queue)
        .await
        .map(|v| !v.is_empty())
        .unwrap_or(false);
    if has_new_input || has_pending_steers || has_pending_queue {
        abandon_cancelled_tasks(session, &store, &cancelled).await;
        return;
    }
    let cancel = session.cancel.clone();
    let mut backfill: Vec<ContentBlock> = Vec::with_capacity(cancelled.len());
    for task in &cancelled {
        if let Some(c) = &cancel {
            if c.is_cancelled() {
                break;
            }
        }
        let outcome = replay_child(
            store.clone(),
            task,
            &session.config,
            &session.client,
            &session.working_dir,
            cancel.as_ref(),
        )
        .await;
        let (text, ok) = match outcome {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    task_id = %task.task_id,
                    child = %task.child_session_id,
                    error = %e,
                    "cancelled subagent replay failed; backfilling an error result"
                );
                (format!("subagent resume failed: {e:#}"), false)
            }
        };
        let _ = store.complete_subagent_task(&task.task_id, &text, ok).await;
        backfill.push(ContentBlock::ToolResult {
            tool_use_id: task.task_id.clone(),
            content: text,
            is_error: !ok,
            images: Vec::new(),
        });
    }
    if backfill.is_empty() {
        return;
    }
    let tool_msg = Message {
        id: crate::runner::new_id(),
        role: Role::Tool,
        blocks: backfill,
        model: None,
        agent: None,
        usage: MessageUsage::default(),
        created_at: now_ms(),
        synthetic: false,
    };
    session.record(tool_msg).await;
}

/// Backfill terminal "cancelled" tool_results for subagent tasks that were
/// interrupted by a user steer (redirect), WITHOUT re-running the children.
/// Each task's `tool_use` gets a terminal error `tool_result` so the transcript
/// stays well-formed (no dangling ids that providers reject with HTTP 400), and
/// the task is marked Failed so `replay_cancelled_tasks` never picks it up
/// again. Used when the user steers or submits new input mid-subagent: they
/// want to move on, not resume the interrupted child.
async fn abandon_cancelled_tasks(
    session: &mut SessionState,
    store: &Arc<dyn Store>,
    tasks: &[SubagentTaskRecord],
) {
    const MSG: &str = "cancelled: the user moved on to new input (redirect).";
    let mut backfill: Vec<ContentBlock> = Vec::with_capacity(tasks.len());
    for task in tasks {
        let _ = store
            .complete_subagent_task(&task.task_id, MSG, false)
            .await;
        backfill.push(ContentBlock::ToolResult {
            tool_use_id: task.task_id.clone(),
            content: MSG.to_string(),
            is_error: true,
            images: Vec::new(),
        });
        tracing::info!(
            task_id = %task.task_id,
            child = %task.child_session_id,
            "abandoning cancelled subagent (user moved on) instead of replaying"
        );
    }
    let tool_msg = Message {
        id: crate::runner::new_id(),
        role: Role::Tool,
        blocks: backfill,
        model: None,
        agent: None,
        usage: MessageUsage::default(),
        created_at: now_ms(),
        synthetic: false,
    };
    session.record(tool_msg).await;
}

/// Resume a single child task and run it to completion with an empty prompt
/// ("continue"). The child's continuation messages and events are persisted to
/// its own session, mirroring `run_subagent`. Returns `(result_text, ok)`.
///
/// Bounded by `config.replay_timeout()` and abortable via `parent_cancel` so
/// recovery can never freeze the parent indefinitely: an interrupted subagent
/// is re-run, but a wedged child (slow-but-alive LLM stream, near-doom loop)
/// is cut off and its partial result backfilled instead.
async fn replay_child(
    store: Arc<dyn Store>,
    task: &SubagentTaskRecord,
    config: &Config,
    client: &Arc<dyn ChatStream>,
    working_dir: &Path,
    parent_cancel: Option<&CancellationToken>,
) -> Result<(String, bool)> {
    // Children never carry subagent tasks of their own (no `task` tool), so
    // resume()'s stuck-task path is a no-op here; its dangling-`tool_use`
    // reconciliation correctly patches a child interrupted mid-tool-call.
    let mut child = resume(
        store.clone(),
        &task.child_session_id,
        config.clone(),
        client.clone(),
        working_dir.to_path_buf(),
    )
    .await?;

    // Same force-off as `run_subagent`: a replayed child session runs a
    // scoped task, never autopilot passes — even when the resumed config
    // carries `autopilot.mode = "ap"|"review"` for the parent.
    child.config.autopilot.mode = opencoder_core::ApMode::Off;

    // resume() leaves `cancel` as `None`; without a token the run loop's
    // interrupt check is skipped entirely, so a parent cancel or the replay
    // timeout could never break the child out of its loop. Install one.
    let child_token = CancellationToken::new();
    child.cancel = Some(child_token.clone());

    // Incremental child-event persistence (same ordered-flusher pattern as
    // `run_subagent`): events reach the DB as they are produced so a second
    // interruption still leaves partial child progress reconstructable.
    let child_id = task.child_session_id.clone();
    let (ev_tx, ev_rx) =
        tokio::sync::mpsc::channel::<SessionEventRecord>(crate::event_sink::CAPACITY);
    let flush_store = Some(store.clone());
    // Batched, lossless drain (shared with TUI/web/subagent surfaces).
    let flusher = tokio::spawn(crate::event_sink::run_flusher(flush_store, ev_rx));
    let registry = crate::tools::registry();

    // Overall replay deadline. Recovery must not block the user indefinitely:
    // the only internal cap on `run_with_registry` is the per-LLM-turn idle
    // timeout, so a child could otherwise run across many turns for hours.
    let run_dur = config.replay_timeout();

    // Boxed to break the run_with_registry -> replay_cancelled_tasks ->
    // replay_child -> run_with_registry recursion (children hold no task tool,
    // so replay_cancelled_tasks is a no-op there, but the type must be finite).
    let run = Box::pin(crate::runner::run_with_registry(
        &mut child,
        String::new(),
        Vec::new(),
        &registry,
        move |cev| {
            let rec = SessionEventRecord {
                session_id: child_id.clone(),
                kind: cev.coarse_kind(),
                payload: serde_json::to_value(&cev).unwrap_or(serde_json::Value::Null),
                ts: now_ms(),
                seq: None,
                sse_kind: Some(cev.sse_kind().to_string()),
            };
            // `try_send` is sync/non-blocking; on a full channel (slow-DB
            // backpressure) delta fragments are silently dropped (display-only
            // — the child's authoritative text lands via its messages append),
            // everything else is logged and dropped.
            match ev_tx.try_send(rec) {
                Ok(()) => {}
                Err(tokio::sync::mpsc::error::TrySendError::Full(rec))
                    if rec.kind == EventKind::TextDelta => {}
                Err(e) => {
                    tracing::warn!(error = %e, "replay: child event channel full/closed, dropping event");
                }
            }
        },
    ));

    // Race the child run against the replay deadline and parent cancellation.
    // Whichever non-run branch wins cancels the child token (graceful stop at
    // the next turn boundary) and the `run` future is dropped (hard cancel of
    // any in-flight LLM/tool call). The authoritative child text is already
    // persisted via `session.record()`, so partial completion is recoverable.
    let res = tokio::select! {
        biased;
        _ = async {
            match parent_cancel {
                Some(t) => t.cancelled().await,
                None => std::future::pending::<()>().await,
            }
        } => {
            child_token.cancel();
            tracing::info!(
                task_id = %task.task_id,
                child = %task.child_session_id,
                "replay cancelled by parent during recovery"
            );
            Err(anyhow!("replay cancelled"))
        }
        _ = tokio::time::sleep(run_dur) => {
            child_token.cancel();
            tracing::warn!(
                task_id = %task.task_id,
                child = %task.child_session_id,
                timeout_secs = run_dur.as_secs(),
                "replay timed out during recovery; backfilling partial result"
            );
            Err(anyhow!("replay timed out after {}s", run_dur.as_secs()))
        }
        r = run => r,
    };

    // The callback owned `ev_tx`; once `run_with_registry` returns (or is
    // cancelled) the closure is dropped, closing the channel so the flusher
    // drains and exits. Bound the wait so a wedged DB flusher cannot freeze
    // recovery; the authoritative child text is already persisted.
    let _ = tokio::time::timeout(Duration::from_secs(30), flusher).await;

    let ok = res.is_ok();
    let text = child
        .messages
        .iter()
        .rev()
        .find(|m| m.role == Role::Assistant)
        .map(|m| m.text())
        .unwrap_or_default();
    Ok((text, ok))
}

/// Generate a short title from the first user/assistant exchange, using the
/// small model when configured. Persists the title to the store. Non-fatal:
/// errors are logged and swallowed.
pub async fn generate_title(session: &SessionState) {
    if session.store.is_none() {
        return;
    }
    let store = session.store.clone().unwrap();
    if let Err(e) = generate_title_inner(session, &store).await {
        tracing::warn!(session_id = %session.id, error = %e, "title generation failed");
    }
}

async fn generate_title_inner(session: &SessionState, store: &Arc<dyn Store>) -> Result<()> {
    let msgs = lower_messages(&session.messages);
    let req = ChatRequest {
        model: session.config.small_model_or_primary().to_string(),
        messages: msgs,
        tools: Vec::new(),
        tool_choice: None,
        temperature: Some(0.3),
        max_tokens: Some(64),
        reasoning_effort: None,
        cache_salt: crate::cache_salt_for(session),
    };
    let mut rx = session.client.chat_stream(req).context("title llm call")?;
    let mut text = String::new();
    while let Some(ev) = rx.recv().await {
        match ev {
            LlmEvent::TextDelta(t) => text.push_str(&t),
            LlmEvent::Completed { text: t, .. } => {
                if !t.is_empty() {
                    text = t;
                }
                break;
            }
            LlmEvent::Retrying { .. } => {
                // Mid-stream retry: drop deltas so the two attempts aren't
                // concatenated; the final `Completed` overwrites `text`.
                text.clear();
            }
            LlmEvent::Error(e) => return Err(anyhow!(e)),
            _ => {}
        }
    }
    let title: String = text.trim().chars().take(80).collect();
    if title.is_empty() {
        return Ok(());
    }
    store
        .update_session(
            &session.id,
            &opencoder_store::SessionPatch {
                title: Some(title),
                updated_at: Some(opencoder_core::message::now_ms()),
                ..Default::default()
            },
        )
        .await?;
    Ok(())
}
