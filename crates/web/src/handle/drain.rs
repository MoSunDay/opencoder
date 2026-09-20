//! Session drain lifecycle and its execution configuration.
use super::*;
use opencoder_session::{resume_and_replay as resume_session, run};

pub(super) struct DrainContext {
    pub workdir: std::path::PathBuf,
    pub config_home: Option<std::path::PathBuf>,
    pub config: Config,
}

/// Drive the session runner to completion, broadcasting events.
pub(super) async fn drain_to_completion(
    handles: HandleMap,
    store: Arc<dyn Store>,
    session_id: &str,
    client: Arc<dyn ChatStream>,
    context: DrainContext,
    handle: Arc<SessionHandle>,
) {
    let DrainContext {
        workdir,
        config_home,
        mut config,
    } = context;
    let guard = DrainGuard {
        handle: handle.clone(),
    };
    let mut rx_guard = CmdRxGuard {
        handle: handle.clone(),
        rx: handle.cmd_rx.lock().map(|mut g| g.take()).ok().flatten(),
    };

    {
        let ov = handle.overrides.lock().await;
        if let Some(a) = &ov.agent {
            config.agent.default = a.clone();
        }
        if let Some(m) = &ov.model {
            config.model = m.clone();
        }
    }

    let cancel_token = handle.cancel.lock().await.clone();
    let mut session = match resume_session(
        store.clone(),
        session_id,
        config.clone(),
        client.clone(),
        workdir.clone(),
        Some(cancel_token),
    )
    .await
    {
        Ok(s) => s,
        Err(e) => {
            warn!(session_id, error = %e, "drain: cannot resume (session row missing?)");
            // TUI worker contract (worker.rs): a drain that cannot even start
            // still owes its SSE subscribers a terminal frame. Without this
            // broadcast the stream hangs open with no Error and no Done while
            // `draining` resets below — a silently dead UI.
            broadcast_persist_event(
                &store,
                &handle,
                session_id,
                SessionEvent::Error(format!("drain: resume failed: {e:#}")),
            )
            .await;
            let mut map = handles.lock().await;
            // Only reclaim the map entry when nobody is listening. Live SSE
            // subscribers still hold THIS handle's broadcast receiver: removing
            // the entry would orphan them (a later prompt creates a NEW
            // handle/tx they never receive, and their eventual
            // `release_events_subscriber` would decrement that fresh instance's
            // counter — underflow). The check runs under the map lock, which is
            // the same lock every subscribe/increment takes, so a zero count is
            // authoritative. With subscribers attached, keep the entry: the
            // normal eviction path (last subscriber leaves while idle) reclaims
            // it later. Also only remove when the entry is still THIS instance —
            // it may have been deleted + recreated meanwhile (e.g. DELETE).
            let still_current = map.get(session_id).is_some_and(|h| Arc::ptr_eq(h, &handle));
            let live =
                handle.subscribers.load(Ordering::SeqCst) > 0 || handle.tx.receiver_count() > 0;
            if live {
                warn!(
                    session_id,
                    subscribers = handle.subscribers.load(Ordering::SeqCst),
                    "drain: resume failed but SSE subscribers remain; keeping handle"
                );
            } else if still_current {
                map.remove(session_id);
            }
            return;
        }
    };
    session.cancel = Some(handle.cancel.lock().await.clone());
    session.child_turn_cancels = handle.child_turn_cancels.clone();
    session.child_steer_gates = handle.child_steer_gates.clone();
    session.child_cancels = handle.child_cancels.clone();
    session.turn_cancel = Some(handle.turn_cancel.clone());
    // Rebind the question hub to the handle's stable instance and mark a web
    // listener as attached: `resume_session` builds a fresh (unattached) hub,
    // which would make every `question` tool call fall back to
    // NO_LISTENER_REPLY. The runner's registry (runner/registry.rs) builds the
    // `question` tool from `session.question_hub` inside `run()`, which is
    // invoked AFTER this swap — so the tool gets exactly this handle's hub,
    // letting the /questions endpoints answer it mid-turn.
    session.question_hub = handle.question_hub.clone();
    handle.question_hub.attach();

    // 广播句柄克隆进回调：`broadcast_evt` 同时写 ring（pre-subscribe gap
    // 桥接）与直播通道，取代裸 `tx.send`。
    let bcast = Arc::clone(&handle);
    let sid = session_id.to_string();
    let (sink, flusher) =
        opencoder_session::spawn_event_flusher(Some(store.clone()), session_id.to_string());
    // Zero-resubmit: a failed drain NEVER auto-resubmits pending inputs and
    // fires no additional LLM requests. Pending steer/queue rows stay in the
    // store (the admit POST's durable promise) and are consumed by the NEXT
    // successful drain instead of being silently retried inside this failing
    // one. Deliberate semantic change of the former bounded drain-restart
    // loop; the drops below run exactly once, after this single attempt.
    let mut run_emitted_error = false;
    let result = run(&mut session, String::new(), |ev| {
        if matches!(ev, SessionEvent::Error(_)) {
            run_emitted_error = true;
        }
        let (sse, _kind) = sse_from_session_event(&sid, &ev);
        bcast.broadcast_evt(sse);
        let _ = sink.push(&ev);
    })
    .await;

    // Terminal-frame guarantee BEFORE drain commands run: a failed run must
    // surface an `error` frame even when the runner emitted none (see
    // ensure_run_error_frame). Emitted first so no later Done can mask it.
    ensure_run_error_frame(&store, &handle, &sid, &result, run_emitted_error).await;

    // Apply endpoint-forwarded drain commands (autopilot/annotation/...)
    // once the run settles.
    process_drain_cmds(
        &mut session,
        &mut rx_guard,
        &handle,
        &sink,
        &sid,
        &workdir,
        config_home.as_deref(),
    )
    .await;

    // Best-effort title generation after the FIRST successful completion of a
    // drain (mirrors `crates/cli/src/run.rs`): runs while the event sink is
    // still alive but after the run loop breaks, bounded at 30 s so a hanging
    // small-model endpoint can never wedge teardown. `result.is_ok()` gates
    // it to successful runs, and the title check inside makes it once-only
    // across a session's many drains. Failures only log.
    crate::handle_questions::maybe_generate_title(&store, &session, result.is_ok()).await;

    drop(sink);
    if let Err(e) = flusher.await {
        warn!(session_id, error = %e, "final event flush failed");
    }
    // flusher 已排空：此刻 ring 中所有条目必然已落库（或已按丢批策略放弃），
    // 回放可全覆盖，清空 ring。否则一次空闲期重连（after=最新 seq、回放为
    // 空）会把上一 turn 的广播尾巴当「新事件」整体重发。ring 的职责只是
    // 覆盖 flusher 攒批滞后 + 订阅延迟，drain 收束后即失去意义。
    if let Ok(mut ring) = handle.recent.lock() {
        ring.clear();
    }
    drop(rx_guard);
    if let Err(e) = result {
        warn!(session_id, error = %e, "drain ended with error");
    }
    // Keep `draining` true through every teardown step. Clearing it before
    // the event flusher finished and `cmd_rx` was restored opened a tail race:
    // an idle-only mutation (or a fresh drain) could start against a task that
    // had not actually relinquished all of its per-session resources yet.
    drop(guard);
}
