use super::*;

/// Build the "Valid options" list for a subagent_type rejection error, gated
/// by agent kind and the `tools_subagent` capability. Plan mode omits 'build';
/// a disabled capability omits 'tools'.
pub(super) fn valid_subagent_options(plan: bool, tools_on: bool) -> String {
    let mut parts: Vec<&str> = vec!["'explore' (read-only)"];
    if !plan {
        parts.push("'build' (full tools)");
    }
    if tools_on {
        parts.push("'tools' (browser/computer-use)");
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
    let tools_on = parent.config.capabilities.tools_subagent_enabled();
    // 'tools' umbrella subagent requires its capability switch. Reject before
    // the plan/act classification so the error never advertises 'tools' when
    // the capability is disabled.
    if kind == "tools" && !tools_on {
        return ToolOutput::err(format!(
            "Unknown subagent_type '{kind}'. Valid options: {}",
            valid_subagent_options(plan, tools_on)
        ));
    }
    // Plan mode may only spawn read-only subagents: 'explore' (filesystem) and,
    // when enabled, 'tools' (browser fetch/search + computer-use are read-only
    // w.r.t. the repo). 'build' stays rejected so the model is never told it
    // exists.
    if plan && !matches!(kind.as_str(), "explore" | "tools") {
        return ToolOutput::err(format!(
            "Unknown subagent_type '{kind}'. Valid options: {}",
            valid_subagent_options(plan, tools_on)
        ));
    }
    let agent = match resolve_agent(&kind) {
        Some(a) => a,
        None => {
            return ToolOutput::err(format!(
                "Unknown subagent_type '{kind}'. Valid options: {}",
                valid_subagent_options(plan, tools_on)
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
    let (ev_tx, ev_rx) = tokio::sync::mpsc::unbounded_channel::<SessionEventRecord>();
    let flush_store = child_store.clone();
    // Batched, lossless drain shared with the TUI/web surfaces: deltas are
    // coalesced into one transactional append_events; non-delta events flush
    // pending deltas first; channel close triggers a final flush.
    let flusher = tokio::spawn(crate::event_sink::run_flusher(flush_store, ev_rx));
    let res = Box::pin(run_with_registry(
        &mut child,
        prompt.clone(),
        Vec::new(),
        registry,
        move |cev| {
            // Incremental persist: push to the ordered flusher channel. The
            // callback is sync (cannot await); the channel is unbounded, so
            // send never blocks and can only fail if the flusher has exited
            // (closed) — in which case the single event is logged and dropped.
            if has_store {
                let rec = SessionEventRecord {
                    session_id: child_id_for_cb.clone(),
                    kind: cev.coarse_kind(),
                    payload: serde_json::to_value(&cev).unwrap_or(serde_json::Value::Null),
                    ts: now_ms(),
                    seq: None,
                    sse_kind: Some(cev.sse_kind().to_string()),
                };
                if let Err(e) = ev_tx.send(rec) {
                    tracing::warn!(error = %e, "subagent: child event channel closed, dropping event");
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

    // Detect cancellation: the shared token fired (web interrupt / double-Esc),
    // so the child broke out of its run loop without a real result. Mark the
    // task cancelled and leave the parent tool_use open (no tool_result) so the
    // child can be replayed on the next user turn. run_loop skips recording the
    // tool message when cancelled, keeping this tool_use dangling.
    let cancelled = parent
        .cancel
        .as_ref()
        .map(|c| c.is_cancelled())
        .unwrap_or(false);
    if cancelled {
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
