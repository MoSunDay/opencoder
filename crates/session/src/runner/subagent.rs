use super::*;
use tokio_util::sync::CancellationToken;

/// Build the "Valid options" list for a subagent_type rejection error, gated
/// by agent kind. Plan mode omits 'build' (it is read-only).
pub(super) fn valid_subagent_options(plan: bool) -> String {
    let mut parts: Vec<&str> = vec!["'explore' (read-only)"];
    if !plan {
        parts.push("'build' (full tools)");
    }
    match parts.len() {
        1 => parts[0].to_string(),
        2 => format!("{} or {}", parts[0], parts[1]),
        _ => {
            let (last, rest) = parts.split_last().unwrap();
            format!("{}, or {}", rest.join(", "), last)
        }
    }
}

pub(super) async fn run_subagent(
    input: Value,
    call_id: String,
    parent: &SessionState,
    registry: &HashMap<String, ToolArc>,
    sink: &Sink<'_>,
    activity: tokio::sync::mpsc::Sender<()>,
    timed_out: Arc<std::sync::atomic::AtomicBool>,
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
    let plan = parent.agent.kind == AgentKind::Plan;
    // Plan mode may only spawn read-only subagents: 'explore' (filesystem).
    // 'build' stays rejected so the model is never told it exists.
    if plan && kind != "explore" {
        return ToolOutput::err(format!(
            "Unknown subagent_type '{kind}'. Valid options: {}",
            valid_subagent_options(plan)
        ));
    }
    let agent = match resolve_agent(&kind) {
        Some(a) => a,
        None => {
            return ToolOutput::err(format!(
                "Unknown subagent_type '{kind}'. Valid options: {}",
                valid_subagent_options(plan)
            ));
        }
    };
    let child_session_id = format!("sub-{}", new_id());
    let steer_gate = crate::SubagentSteerGate::new();
    if let Ok(mut map) = parent.child_steer_gates.lock() {
        map.insert(call_id.clone(), steer_gate.clone());
    }
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
    child.steer_gate = Some(steer_gate.clone());
    // Derive a child token from the parent's hard-cancel token. A parent
    // double-Esc (parent cancelled) cascades to the child via the parent-child
    // link; but the child can also be independently cancelled through
    // `parent.child_cancels` (a parent steer) without ending the parent's own
    // run_loop.
    child.cancel = parent.cancel.as_ref().map(|pc| pc.child_token());
    if let Some(ct) = &child.cancel {
        if let Ok(mut map) = parent.child_cancels.lock() {
            map.insert(call_id.clone(), ct.clone());
        }
    }

    // Create and register a turn-level interrupt token for the child. This
    // allows subagent steer to interrupt the current turn without ending the
    // child's run_loop. The token is shared via parent.child_turn_cancels so
    // external code (TUI, web) can fire it by call_id.
    let turn_token: crate::SharedCancel = Arc::new(std::sync::Mutex::new(CancellationToken::new()));
    if let Ok(mut map) = parent.child_turn_cancels.lock() {
        map.insert(call_id.clone(), turn_token.clone());
    }
    child.turn_cancel = Some(turn_token);

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
                summary_images: vec![],
                handoff_seq: None,
                handoff_plan: None,
                skill: None,
                task_type: Some(opencoder_store::TASK_TYPE_SUBAGENT.to_string()),
                requirement: None,
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
    // Incremental child-event persistence: a single flusher task drains a
    // bounded mpsc channel and awaits `append_event` per record in emission
    // order (one consumer → DB seq stays aligned with emission order). Events
    // reach the DB as they are produced, so a hard interruption mid-subagent
    // leaves partial progress persisted (reconstruct_child_view reads
    // events_after(child, 0)) instead of losing everything. The flusher is
    // awaited before return so a normal completion flushes 100% of buffered
    // events.
    let (ev_tx, ev_rx) =
        tokio::sync::mpsc::channel::<SessionEventRecord>(crate::event_sink::CAPACITY);
    let flush_store = child_store.clone();
    // Batched, lossless drain shared with the TUI/web surfaces: deltas are
    // coalesced into one transactional append_events; non-delta events flush
    // pending deltas first; channel close triggers a final flush.
    // The flusher JoinHandle is held in an abort-on-drop guard: a force cancel of
    // the parent drops this future mid-`run_with_registry`, which would drop
    // `flusher` without aborting the task — leaving a detached task holding
    // `Arc<Store>`. The guard aborts on drop unless the normal completion path
    // takes the handle out to await it.
    let mut flusher_guard = FlushAbortOnDrop::new(tokio::spawn(
        crate::event_sink::run_flusher(flush_store, ev_rx),
    ));
    let res = Box::pin(run_with_registry(
        &mut child,
        prompt.clone(),
        Vec::new(),
        registry,
        move |cev| {
            // Signal forward progress so the parent's idle-timeout watchdog
            // (Phase-1 reset loop in execute.rs) resets its deadline. Every
            // SessionEvent counts as activity: a stalled child is one that
            // produces *no* events at all. `try_send` is non-blocking and the
            // signal is idempotent, so a full/closed channel is silently
            // dropped (only the most recent real activity matters).
            let _ = activity.try_send(());
            // Incremental persist: push to the ordered flusher channel. The
            // callback is sync (cannot await); `try_send` is non-blocking, so
            // it can only fail if the channel is full (slow-DB backpressure)
            // or the flusher has exited (closed). Under backpressure delta
            // fragments are silently dropped (display-only); everything else
            // is logged and dropped — same loss semantics as `EventSink::push`.
            if has_store {
                let rec = SessionEventRecord {
                    session_id: child_id_for_cb.clone(),
                    kind: cev.coarse_kind(),
                    payload: serde_json::to_value(&cev).unwrap_or(serde_json::Value::Null),
                    ts: now_ms(),
                    seq: None,
                    sse_kind: Some(cev.sse_kind().to_string()),
                };
                match ev_tx.try_send(rec) {
                    Ok(()) => {}
                    // Delta fragments are safe to drop under backpressure —
                    // the child's authoritative text lands via its messages
                    // append, so only a momentary display gap results.
                    Err(tokio::sync::mpsc::error::TrySendError::Full(rec))
                        if rec.kind == opencoder_store::EventKind::TextDelta => {}
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            "subagent: child event channel full/closed, dropping event"
                        );
                    }
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
    // Normal completion: take the handle out of the guard (so it is NOT aborted
    // on drop) and await it with the existing bounded timeout. The channel is
    // already closed here (the callback that owns `ev_tx` dropped when
    // `run_with_registry` returned), so the flusher drains remaining events and
    // exits; the timeout is a safety net for a pathologically slow final flush.
    if let Some(flusher) = flusher_guard.take() {
        let _ = tokio::time::timeout(std::time::Duration::from_secs(30), flusher).await;
    }

    // Close admission before removing the runtime registries. A store write
    // that reserved before a forced close will fail commit and roll itself
    // back instead of leaving a permanently pending child steer.
    steer_gate.force_close();
    if let Ok(mut map) = parent.child_steer_gates.lock() {
        map.remove(&call_id);
    }
    // Remove the turn-cancel and cancel tokens from the parent's registries
    // now that the child has finished.
    if let Ok(mut map) = parent.child_turn_cancels.lock() {
        map.remove(&call_id);
    }
    if let Ok(mut map) = parent.child_cancels.lock() {
        map.remove(&call_id);
    }

    // Detect cancellation: the child's hard-cancel token fired. This happens in
    // two scenarios that must be handled differently:
    //
    //  1. Parent STEER (TUI `>` / web POST /prompt with running children): the
    //     user redirected the parent away from this subagent. Only the child
    //     token fired -- the parent's own `cancel` is intact. Make the task
    //     TERMINAL (Failed) and record a real tool_result so the transcript
    //     stays well-formed and the task is never silently replayed on resume.
    //  2. Parent HARD-ABORT (double-Esc / web POST /interrupt): the parent's
    //     `cancel` fired and cascaded to the child token. Keep the task
    //     Cancelled and leave the parent tool_use dangling (no tool_result) so
    //     run_loop skips recording the tool message and the child can be
    //     replayed on the next user turn.
    //
    // The two are distinguishable because `child.cancel` is a `child_token()`
    // of the parent's cancel: a steer cancels only the child token, while a
    // hard-abort cancels the parent token (which the child observes too).
    let cancelled = child
        .cancel
        .as_ref()
        .map(|c| c.is_cancelled())
        .unwrap_or(false);
    if cancelled {
        // D1: distinguish a *timeout* from a parent steer. Both fire the
        // child's hard-cancel token (execute.rs fires `fire_child_cancel`
        // itself on timeout), so `child.cancel.is_cancelled()` is true in
        // either case and the parent's own cancel is intact in both. The
        // shared `timed_out` flag (set by execute.rs synchronously before the
        // Phase-2 await) is the only way to tell them apart. Check it FIRST so
        // a timeout is reported as a timeout, not mislabelled as a steer.
        if timed_out.load(std::sync::atomic::Ordering::SeqCst) {
            const TIMEOUT_MSG: &str = "cancelled: timed out";
            if let Some(store) = &parent.store {
                let _ = store.cancel_subagent_task(&call_id).await;
            }
            emit(
                sink,
                SessionEvent::SubagentEnd {
                    id: call_id.clone(),
                    ok: false,
                    cancelled: true,
                    summary: TIMEOUT_MSG.to_string(),
                },
            );
            return ToolOutput::err(TIMEOUT_MSG);
        }
        let parent_aborted = parent
            .cancel
            .as_ref()
            .map(|c| c.is_cancelled())
            .unwrap_or(false);
        if !parent_aborted {
            const STEER_MSG: &str = "cancelled: redirected by parent steer";
            if let Some(store) = &parent.store {
                let _ = store
                    .complete_subagent_task(&call_id, STEER_MSG, false)
                    .await;
            }
            emit(
                sink,
                SessionEvent::SubagentEnd {
                    id: call_id.clone(),
                    ok: false,
                    cancelled: true,
                    summary: STEER_MSG.to_string(),
                },
            );
            return ToolOutput::err(STEER_MSG);
        }
        if let Some(store) = &parent.store {
            let _ = store.cancel_subagent_task(&call_id).await;
        }
        emit(
            sink,
            SessionEvent::SubagentEnd {
                id: call_id.clone(),
                ok: false,
                cancelled: true,
                summary: "(cancelled)".to_string(),
            },
        );
        return ToolOutput::err("cancelled");
    }

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
            cancelled: false,
            summary: format!("({} tool calls) {}", child_tools, summary_preview),
        },
    );
    if ok {
        ToolOutput::ok(text)
    } else {
        // Surface the real failure reason instead of an opaque banner. The
        // child's `run_loop` returns Err for hard failures (LLM error, stream
        // ended without completion, panic); `text` holds whatever final
        // assistant text the child produced (often empty on a hard crash).
        // Combine both so the parent model can react to the actual cause.
        let detail = match res.as_ref().err() {
            Some(e) => {
                let mut s = format!("subagent failed: {e}");
                if !text.is_empty() {
                    s.push_str("\n\n");
                    s.push_str(&text);
                }
                s
            }
            None => {
                if text.is_empty() {
                    "subagent failed".to_string()
                } else {
                    text
                }
            }
        };
        ToolOutput::err(detail)
    }
}

/// RAII guard that aborts a spawned flusher task on drop. The subagent runner
/// stores its event-flusher `JoinHandle` here; on the normal completion path
/// the handle is `take()`n out and awaited, so the guard drops empty. If the
/// owning future is cancelled first, the guard still holds the handle and
/// aborts the detached task instead of leaking it (it would otherwise hold an
/// `Arc<Store>` until it noticed its channel had closed).
struct FlushAbortOnDrop {
    handle: Option<tokio::task::JoinHandle<()>>,
}

impl FlushAbortOnDrop {
    fn new(handle: tokio::task::JoinHandle<()>) -> Self {
        Self {
            handle: Some(handle),
        }
    }
    /// Remove and return the inner handle, disarming the guard.
    fn take(&mut self) -> Option<tokio::task::JoinHandle<()>> {
        self.handle.take()
    }
}

impl Drop for FlushAbortOnDrop {
    fn drop(&mut self) {
        if let Some(h) = self.handle.take() {
            h.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    /// Dropping a `FlushAbortOnDrop` that still holds its handle must abort
    /// the wrapped task — a force-cancel drops the guard mid-flight.
    #[tokio::test]
    async fn flush_guard_aborts_task_on_drop() {
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();
        let handle = tokio::spawn(async move {
            loop {
                counter_clone.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        });

        {
            let _guard = FlushAbortOnDrop::new(handle);
            tokio::time::sleep(Duration::from_millis(20)).await;
            assert!(
                counter.load(Ordering::SeqCst) > 0,
                "task should be incrementing before drop"
            );
        } // guard dropped → abort()

        // Give the runtime a moment to process the abort.
        tokio::time::sleep(Duration::from_millis(15)).await;
        let baseline = counter.load(Ordering::SeqCst);
        // An un-aborted task would increment ~6x in this window.
        tokio::time::sleep(Duration::from_millis(30)).await;
        let after = counter.load(Ordering::SeqCst);
        assert_eq!(
            baseline, after,
            "task must stop incrementing after abort-on-drop (was {baseline}, now {after})"
        );
    }

    /// `take()` disarms the guard so dropping it does NOT abort. The handle
    /// is returned and can be awaited to completion.
    #[tokio::test]
    async fn flush_guard_take_disarms_and_task_completes() {
        let handle = tokio::spawn(async {});
        let mut guard = FlushAbortOnDrop::new(handle);
        tokio::time::sleep(Duration::from_millis(5)).await;
        let taken = guard.take().expect("handle must be present after construction");
        drop(guard); // disarmed — must NOT abort
        // Awaiting must succeed — if take() had failed and drop(guard) had
        // aborted the task, this would panic with a JoinError.
        taken.await.unwrap();
    }
}
