use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use opencoder_core::{ToolArc, ToolContext, ToolOutput};
use opencoder_llm::tool_call::CompletedToolCall;
use opencoder_store::Store;
use tokio_util::sync::CancellationToken;

use crate::{SessionEvent, SessionState, SharedCancel};

use super::event::{Sink, MAX_OUTPUT};
use super::steer::{await_cancel, await_turn_cancel};
use super::subagent::run_subagent;

/// Which interrupt/timeout signal won the Phase-1 race against a running
/// subagent. Determines the grace-drain behaviour and the synthesized error
/// message if the subagent cannot clean up in time.
enum TaskSignal {
    HardCancel,
    TurnCancel,
    Timeout,
}

/// Safety-net timeout for a single leaf-tool execution. Prevents a hung tool
/// (e.g. an ssh_pty tmux call that never returns, or a hung tool whose future
/// never resolves) from freezing the
/// run loop forever. Generous enough that legitimate long-running tools are
/// unaffected. `bash` is exempt entirely: it passes `None` and relies on its
/// own internal foreground timeout (`tools::bash::BASH_TIMEOUT_SECS`): when
/// exceeded the command is moved to the background rather than killed, so the
/// two deadlines never race. The `task` subagent early-returns before this guard
/// is reached (a child session may legitimately run for many minutes).
/// Pairs with the per-read
/// LLM idle timeout (`DEFAULT_READ_TIMEOUT`); both are last-resort guards,
/// not expected to fire in normal operation.
pub(crate) const DEFAULT_TOOL_TIMEOUT: Duration = Duration::from_secs(600);

/// Extract a human-readable message from a caught panic payload
/// (`Box<dyn Any + Send>`). Panics are typically constructed from `&str` or
/// `String`; anything else degrades to a generic note.
pub(super) fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(m) = payload.downcast_ref::<&'static str>() {
        (*m).to_string()
    } else if let Some(m) = payload.downcast_ref::<String>() {
        m.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

pub(super) async fn execute_call(
    tc: &CompletedToolCall,
    session: &SessionState,
    registry: &HashMap<String, ToolArc>,
    sink: &Sink<'_>,
) -> ToolOutput {
    let timeout = leaf_tool_timeout(&tc.name);
    execute_call_with_timeout(tc, session, registry, sink, timeout).await
}

/// Wall-clock timeout the run loop wraps a single leaf tool in. Decides the
/// safety-net fuse per tool name:
///
/// - `bash` / `question` → `None` (exempt). Bash runs its own internal deadline
///   (`tools::bash::BASH_TIMEOUT_SECS`) that hands long-running commands to the
///   background rather than killing them. Exempting it here keeps the two
///   deadlines from racing.
/// - `read` / `edit` / `search` → the same budget as bash
///   (`BASH_TIMEOUT_SECS`). These are local, fast tools; a hang past ~2 min is a
///   real problem. Unlike bash, on expiry they are simply cancelled with a
///   "timed out" message (the generic leaf-tool path below drops the future —
///   no background continuation, no handoff).
/// - everything else → [`DEFAULT_TOOL_TIMEOUT`] (the 10-minute net). `task`
///   early-returns before this is reached.
///
/// Pure (no state) so the routing is directly unit-testable.
pub(crate) fn leaf_tool_timeout(name: &str) -> Option<Duration> {
    match name {
        // `question` waits for a human answer: wall-clock budgeting it
        // would cut off slow users. Cancel (double-Esc / turn interrupt)
        // remains the only way out, same as bash.
        "bash" | "question" => None,
        "read" | "edit" | "search" => {
            Some(Duration::from_secs(crate::tools::bash::BASH_TIMEOUT_SECS))
        }
        _ => Some(DEFAULT_TOOL_TIMEOUT),
    }
}

/// Like [`execute_call`] but with an injectable timeout, so the safety net is
/// unit-testable with a tiny timeout instead of waiting the full 10 minutes.
pub(super) async fn execute_call_with_timeout(
    tc: &CompletedToolCall,
    session: &SessionState,
    registry: &HashMap<String, ToolArc>,
    sink: &Sink<'_>,
    timeout: Option<Duration>,
) -> ToolOutput {
    if tc.name == "task" {
        // The subagent runs as a child session and may legitimately take many
        // minutes, so it is exempt from the leaf-tool `DEFAULT_TOOL_TIMEOUT`.
        // It still gets its own (generous) deadline + the cancel guard so a
        // wedged child cannot freeze the run loop forever, and an interrupt is
        // honored promptly. Early-returns: the `task` tool never reaches the
        // generic registry-dispatch path below.
        //
        // Two-phase select: Phase 1 races the subagent against the interrupt/
        // timeout signals. If a signal wins, Phase 2 does NOT drop the future —
        // it gives the subagent a grace window to run its cleanup path (mark the
        // task Cancelled, emit SubagentEnd, prune its registry entries). Dropping
        // the future here (the old single-stage `select!`) skipped that cleanup,
        // leaving the DB task stuck in Running and the registries polluted,
        // which caused HTTP-400 hangs when the user continued.
        let task_dur = session.config.task_timeout();
        let drain = session.config.subagent_drain();
        // Snapshot the Arc handles the grace-expiry fallback needs. `sub` (and
        // the cancel futures) borrow `session` immutably for their entire
        // lifetime, so we cannot touch `session` while they are alive.
        let store = session.store.clone();
        let child_cancels = session.child_cancels.clone();
        let child_turn_cancels = session.child_turn_cancels.clone();
        let child_steer_gates = session.child_steer_gates.clone();
        let call_id = tc.id.clone();
        // Activity channel: every event the child produces (tool calls, LLM
        // text/reasoning deltas, tool results) is signalled here and *resets*
        // the idle deadline. So the timeout means "no progress for `task_dur`"
        // rather than a single wall-clock cap: a long-running but active
        // subagent is never killed, only a truly stalled step trips it. A small
        // bounded channel + non-blocking `try_send` keeps the signal lossy and
        // cheap (it is idempotent — only the most recent real activity matters).
        let (act_tx, mut act_rx) = tokio::sync::mpsc::channel::<()>(16);
        // D1: shared flag distinguishing a *timeout* (we fired the child's hard
        // token ourselves) from a genuine parent steer (only the child token
        // fired). Both make `child.cancel.is_cancelled()` true in run_subagent's
        // post-run check, so without this signal the timeout would be
        // misreported as "redirected by parent steer". Set synchronously below,
        // before the Phase-2 await, so it is visible to run_subagent's post-run.
        let timed_out = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut sub = Box::pin(run_subagent(
            tc.input.clone(),
            call_id.clone(),
            session,
            registry,
            sink,
            act_tx,
            timed_out.clone(),
        ));
        let mut cancel_fut = std::pin::pin!(await_cancel(session));
        let mut turn_cancel_fut = std::pin::pin!(await_turn_cancel(session));
        let mut deadline = std::pin::pin!(tokio::time::sleep(task_dur));

        // Phase 1: if the subagent finishes naturally, return its output. If a
        // signal fires, fall through to Phase 2. Using `&mut sub` (borrow)
        // instead of `sub` (move) ensures the future is NOT dropped when a
        // signal wins — it survives for Phase 2.
        //
        // This is a loop (not a single `select!`) so child activity can reset
        // the idle deadline repeatedly: as long as the subagent keeps producing
        // events it may run indefinitely; only a stalled step with no event for
        // `task_dur` trips Timeout. Biased ordering — cancel > activity >
        // timeout > sub — ensures a racing activity always wins the reset over
        // a simultaneously-elapsed deadline, avoiding false timeouts at the edge.
        //
        // `activity_alive` disables the activity arm once the sender drops: a
        // closed mpsc receiver resolves with `None` instantly forever, and
        // under biased ordering that would starve `sub` (it would win every
        // poll, preventing the subagent future from ever resolving). The child
        // drops its sender when its run_loop returns (it then just flushes
        // events + writes the DB result), so once the channel closes we let the
        // deadline / sub / cancel arms settle the race.
        let mut activity_alive = true;
        let signal: TaskSignal = loop {
            tokio::select! {
                biased;
                _ = &mut cancel_fut => break TaskSignal::HardCancel,
                _ = &mut turn_cancel_fut => break TaskSignal::TurnCancel,
                res = act_rx.recv(), if activity_alive => match res {
                    Some(()) => deadline.as_mut().reset(tokio::time::Instant::now() + task_dur),
                    None => activity_alive = false,
                },
                _ = &mut deadline => break TaskSignal::Timeout,
                o = &mut sub => return o,
            }
        };

        // Phase 2: the child's hard-cancel token is a child_token() of the
        // parent's cancel, so a hard interrupt already cascaded and the child
        // breaks at its next check-point. A turn-level interrupt uses an
        // independent token (not cascaded), so fire it explicitly so the child
        // can drain promptly instead of waiting for its LLM turn to finish.
        // A timeout fires no token of its own in Phase 1, so fire the child's
        // hard-cancel here so it stops promptly instead of running blind
        // through the drain window (and silently marking itself Completed).
        if matches!(signal, TaskSignal::TurnCancel) {
            crate::fire_child_turn_cancel(&child_turn_cancels, &call_id);
        }
        if matches!(signal, TaskSignal::Timeout) {
            // D1: flag this as a *timeout*, not a parent steer, before the
            // Phase-2 await so run_subagent's post-run cancelled branch
            // reports the right summary + DB status.
            timed_out.store(true, std::sync::atomic::Ordering::SeqCst);
            crate::fire_child_cancel(&child_cancels, &call_id);
        }
        return match tokio::time::timeout(drain, &mut sub).await {
            // Subagent finished its cleanup: task is Cancelled (or Completed),
            // SubagentEnd was emitted, registries pruned. Return its result.
            Ok(o) => {
                // On timeout the child's cleanup may have marked the task
                // Completed or Failed (it had no way to know a timeout
                // occurred). Override to Cancelled so the status reflects
                // reality, and surface the timeout error to the parent.
                if matches!(signal, TaskSignal::Timeout) {
                    if let Some(store) = &store {
                        let _ = store.cancel_subagent_task(&call_id).await;
                    }
                    return ToolOutput::err(format!(
                        "subagent timed out after {} without completing",
                        fmt_dur(task_dur)
                    ));
                }
                o
            }
            // Grace expired: the child is wedged. Force the task to Cancelled
            // so it is never stuck Running, prune the stale entries, and emit a
            // terminal SubagentEnd so the UI clears the subagent panel.
            Err(_elapsed) => {
                force_cancel_subagent(
                    store,
                    child_cancels,
                    child_turn_cancels,
                    child_steer_gates,
                    sink,
                    &call_id,
                )
                .await;
                match signal {
                    TaskSignal::Timeout => ToolOutput::err(format!(
                        "subagent timed out after {} without completing",
                        fmt_dur(task_dur)
                    )),
                    TaskSignal::TurnCancel => ToolOutput::err("turn interrupted"),
                    TaskSignal::HardCancel => ToolOutput::err("interrupted"),
                }
            }
        };
    }

    // Sandbox-mode execution gate: unadmitted tools AND mutating bash are
    // refused with a model-visible denial (names the mode, forbids retry,
    // points at `/act`) so the model stops attempting writes instead of
    // looping; see bash_guard::gate for the policy.
    if let Some(denial) = crate::bash_guard::gate(
        &session.agent.kind,
        &tc.name,
        tc.input.get("command").and_then(|v| v.as_str()),
    ) {
        return ToolOutput::err(denial);
    }
    // Latent execution gate (defence in depth): latent tools stay in the
    // registry even while hidden from the schema array, so a hallucinated
    // `question`/`ssh_pty` call would otherwise execute silently. Refuse
    // unless the live skill body still unlocks the tool. Sole exception: an
    // ask on an ATTACHED question hub - a live human channel that renders the
    // card for the user to answer or skip (the tool's own contract), so the
    // ask stays user-visible instead of being dropped on the floor.
    let unlocked = crate::tools::latent::latent_execution_allowed(
        &tc.name,
        session.skill_prompt_cloned().as_deref(),
    );
    let interactive_ask = tc.name == "question" && session.question_hub.is_attached();
    if !unlocked && !interactive_ask {
        return ToolOutput::err(format!(
            "tool `{}` is latent and its owning skill is not active; \
             activate the skill (e.g. `$task-plan`) before calling it",
            tc.name
        ));
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
            let mut turn_cancel_fut = std::pin::pin!(await_turn_cancel(session));
            let exec = tool.execute(tc.input.clone(), &ctx);
            // `None` exempts the tool from the safety net: the deadline future
            // never resolves, so only a cancel or the tool's own completion ends
            // the call. `bash` uses this — it runs in the foreground until it
            // exits and is killed via `/stop`, never timed out.
            let mut deadline: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> =
                match timeout {
                    Some(d) => Box::pin(tokio::time::sleep(d)),
                    None => Box::pin(std::future::pending()),
                };
            tokio::select! {
                biased;
                _ = &mut cancel_fut => ToolOutput::err("interrupted"),
                _ = &mut turn_cancel_fut => ToolOutput::err("turn interrupted"),
                _ = &mut deadline => ToolOutput::err(format!(
                    "tool `{}` timed out after {} without producing a result",
                    tc.name,
                    fmt_dur(timeout.unwrap_or_default())
                )),
                o = exec => o.unwrap_or_else(|e| ToolOutput::err(format!("{e:#}"))),
            }
        }
        None => ToolOutput::err(format!("unknown tool: {}", tc.name)),
    }
}

/// Force a wedged subagent task into a terminal Cancelled state when the grace
/// drain window expires. The subagent's own cleanup path did not run in time,
/// so we replicate its critical side-effects: mark the DB task Cancelled,
/// remove the stale registry entries (keyed by call_id, so they are harmless
/// but pruning keeps the maps tidy), and emit a terminal SubagentEnd so the UI
/// clears the subagent panel. The caller synthesizes the ToolOutput message.
async fn force_cancel_subagent(
    store: Option<Arc<dyn Store>>,
    child_cancels: Arc<Mutex<HashMap<String, CancellationToken>>>,
    child_turn_cancels: Arc<Mutex<HashMap<String, SharedCancel>>>,
    child_steer_gates: Arc<Mutex<HashMap<String, Arc<crate::SubagentSteerGate>>>>,
    sink: &Sink<'_>,
    call_id: &str,
) {
    if let Some(store) = &store {
        let _ = store.cancel_subagent_task(call_id).await;
    }
    if let Ok(mut map) = child_cancels.lock() {
        map.remove(call_id);
    }
    if let Ok(mut map) = child_turn_cancels.lock() {
        map.remove(call_id);
    }
    if let Ok(mut map) = child_steer_gates.lock() {
        if let Some(gate) = map.remove(call_id) {
            gate.force_close();
        }
    }
    super::emit(
        sink,
        SessionEvent::SubagentEnd {
            id: call_id.to_string(),
            ok: false,
            cancelled: true,
            summary: "(cancelled)".to_string(),
        },
    );
}

/// Render a duration compactly (seconds when >= 1 s, milliseconds otherwise) so
/// the timeout message reads naturally for both the 10-minute default and the
/// sub-second durations used in tests.
fn fmt_dur(d: Duration) -> String {
    if d.as_secs() >= 1 {
        format!("{}s", d.as_secs())
    } else {
        format!("{}ms", d.as_millis())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use serde_json::json;

    use super::*;
    use crate::SessionEvent;
    use opencoder_core::{resolve_agent, Config, Tool, ToolContext, ToolOutput};
    use opencoder_llm::{ChatStream, MockChatClient};

    /// A tool whose `execute` future never resolves, to exercise the timeout
    /// safety net without depending on a real long-running tool.
    struct HangingTool;

    #[async_trait]
    impl Tool for HangingTool {
        fn name(&self) -> &str {
            "hang"
        }
        fn description(&self) -> &str {
            "never resolves"
        }
        fn parameters(&self) -> serde_json::Value {
            json!({})
        }
        async fn execute(
            &self,
            _input: serde_json::Value,
            _ctx: &ToolContext,
        ) -> anyhow::Result<ToolOutput> {
            std::future::pending::<()>().await;
            unreachable!()
        }
    }

    /// A tool that resolves instantly, to confirm the timeout does not trip
    /// for well-behaved tools.
    struct FastTool;

    #[async_trait]
    impl Tool for FastTool {
        fn name(&self) -> &str {
            "fast"
        }
        fn description(&self) -> &str {
            "resolves immediately"
        }
        fn parameters(&self) -> serde_json::Value {
            json!({})
        }
        async fn execute(
            &self,
            _input: serde_json::Value,
            _ctx: &ToolContext,
        ) -> anyhow::Result<ToolOutput> {
            Ok(ToolOutput::ok("done"))
        }
    }

    fn make_session() -> SessionState {
        SessionState::new(
            "sess-test",
            resolve_agent("act").unwrap(),
            Config::default(),
            Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>,
            std::env::temp_dir().join("opencer-execute-tests"),
        )
    }

    #[tokio::test]
    async fn hung_tool_returns_timeout_error() {
        let session = make_session();
        let registry: HashMap<String, ToolArc> =
            [("hang".to_string(), Arc::new(HangingTool) as ToolArc)]
                .into_iter()
                .collect();
        let mut noop: Box<dyn FnMut(SessionEvent) + Send> = Box::new(|_| {});
        let sink: Sink<'_> = Arc::new(Mutex::new(&mut *noop));
        let tc = CompletedToolCall {
            id: "tc-1".into(),
            name: "hang".into(),
            input: json!({}),
        };
        let out = execute_call_with_timeout(
            &tc,
            &session,
            &registry,
            &sink,
            Some(Duration::from_millis(50)),
        )
        .await;
        assert!(out.is_error);
        assert!(
            out.content.contains("timed out"),
            "expected timeout message, got: {}",
            out.content
        );
    }

    #[tokio::test]
    async fn fast_tool_is_unaffected_by_timeout() {
        let session = make_session();
        let registry: HashMap<String, ToolArc> =
            [("fast".to_string(), Arc::new(FastTool) as ToolArc)]
                .into_iter()
                .collect();
        let mut noop: Box<dyn FnMut(SessionEvent) + Send> = Box::new(|_| {});
        let sink: Sink<'_> = Arc::new(Mutex::new(&mut *noop));
        let tc = CompletedToolCall {
            id: "tc-2".into(),
            name: "fast".into(),
            input: json!({}),
        };
        // A short timeout that would trip if the tool hung; a fast tool must
        // still return its real result, not the timeout error.
        let out = execute_call_with_timeout(
            &tc,
            &session,
            &registry,
            &sink,
            Some(Duration::from_secs(30)),
        )
        .await;
        assert!(!out.is_error);
        assert_eq!(out.content, "done");
    }

    /// A tool that wraps synchronous blocking work in `spawn_blocking` and
    /// sleeps ~200ms, simulating a real search/ls directory scan. Exercises
    /// the turn_cancel interrupt path: with `spawn_blocking` the async worker
    /// thread is free to poll the cancel token, so firing turn_cancel mid-scan
    /// produces "turn interrupted" rather than blocking until completion.
    struct BlockingTool;

    #[async_trait]
    impl Tool for BlockingTool {
        fn name(&self) -> &str {
            "block"
        }
        fn description(&self) -> &str {
            "blocks for 200ms in spawn_blocking"
        }
        fn parameters(&self) -> serde_json::Value {
            json!({})
        }
        async fn execute(
            &self,
            _input: serde_json::Value,
            _ctx: &ToolContext,
        ) -> anyhow::Result<ToolOutput> {
            let out = tokio::task::spawn_blocking(move || -> ToolOutput {
                std::thread::sleep(std::time::Duration::from_millis(200));
                ToolOutput::ok("blocking done")
            })
            .await
            .unwrap_or_else(|e| ToolOutput::err(format!("blocking task failed: {e}")));
            Ok(out)
        }
    }

    // turn_cancel must interrupt a spawn_blocking tool mid-execution. Before the
    // search/ls fix, synchronous tools hijacked the worker thread and the
    // cancel token was never polled. With spawn_blocking the blocking work runs
    // on a dedicated thread, so the async select! can poll turn_cancel and win.
    #[tokio::test]
    async fn turn_cancel_interrupts_blocking_tool() {
        let session = make_session();
        let registry: HashMap<String, ToolArc> =
            [("block".to_string(), Arc::new(BlockingTool) as ToolArc)]
                .into_iter()
                .collect();
        let mut noop: Box<dyn FnMut(SessionEvent) + Send> = Box::new(|_| {});
        let sink: Sink<'_> = Arc::new(Mutex::new(&mut *noop));
        let tc = CompletedToolCall {
            id: "tc-block".into(),
            name: "block".into(),
            input: json!({}),
        };

        // Grab the turn_cancel token before executing.
        let turn_cancel = session.turn_cancel.as_ref().unwrap().clone();

        // Fire turn_cancel concurrently after a short delay (well within the
        // 200ms blocking window).
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            if let Ok(g) = turn_cancel.lock() {
                g.cancel();
            }
        });

        let out = execute_call_with_timeout(
            &tc,
            &session,
            &registry,
            &sink,
            // Generous timeout so only turn_cancel can win.
            Some(Duration::from_secs(10)),
        )
        .await;

        assert!(out.is_error);
        assert!(
            out.content.contains("turn interrupted"),
            "expected 'turn interrupted', got: {}",
            out.content
        );
    }

    // The blocking tool must still complete normally when no cancel fires —
    // spawn_blocking does not break the happy path.
    #[tokio::test]
    async fn blocking_tool_completes_when_not_interrupted() {
        let session = make_session();
        let registry: HashMap<String, ToolArc> =
            [("block".to_string(), Arc::new(BlockingTool) as ToolArc)]
                .into_iter()
                .collect();
        let mut noop: Box<dyn FnMut(SessionEvent) + Send> = Box::new(|_| {});
        let sink: Sink<'_> = Arc::new(Mutex::new(&mut *noop));
        let tc = CompletedToolCall {
            id: "tc-block-ok".into(),
            name: "block".into(),
            input: json!({}),
        };

        let out = execute_call_with_timeout(
            &tc,
            &session,
            &registry,
            &sink,
            Some(Duration::from_secs(10)),
        )
        .await;

        assert!(!out.is_error);
        assert_eq!(out.content, "blocking done");
    }
    /// With `timeout: None` the safety net must never fire: a perpetually-pending
    /// tool stays pending (responds only to a cancel), rather than erroring with a
    /// "timed out" message. This is the bash exemption — bash has its own internal
    /// timeout (BASH_TIMEOUT_SECS) and does not rely on this safety net.
    #[tokio::test]
    async fn none_timeout_never_fires_for_hung_tool() {
        let session = make_session();
        let registry: HashMap<String, ToolArc> =
            [("hang".to_string(), Arc::new(HangingTool) as ToolArc)]
                .into_iter()
                .collect();
        let mut noop: Box<dyn FnMut(SessionEvent) + Send> = Box::new(|_| {});
        let sink: Sink<'_> = Arc::new(Mutex::new(&mut *noop));
        let tc = CompletedToolCall {
            id: "tc-3".into(),
            name: "hang".into(),
            input: json!({}),
        };
        let call = execute_call_with_timeout(&tc, &session, &registry, &sink, None);
        // The call should NOT resolve on its own (no deadline, hung tool). Race it
        // against a short outer deadline and confirm it was still pending.
        if let Ok(out) = tokio::time::timeout(Duration::from_millis(120), call).await {
            panic!("None deadline should never fire; got: {}", out.content);
        }
    }

    /// `force_cancel_subagent` (the grace-expiry fallback) must replicate the
    /// critical side-effects of `run_subagent`'s cleanup: mark the DB task
    /// Cancelled, prune the stale registry entries, and emit SubagentEnd.
    #[tokio::test]
    async fn force_cancel_marks_task_and_prunes_registries() {
        use opencoder_store::{LibsqlStore, Store, SubagentStatus, SubagentTaskRecord};

        let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
        store
            .create_session(&opencoder_store::SessionMeta {
                id: "force-cancel-parent".into(),
                title: None,
                agent: Some("act".into()),
                model: Some("m".into()),

                autopilot_mode: None,
                workdir_hash: None,
                created_at: 0,
                updated_at: 0,
                summary: None,
                summary_seq: None,
                summary_images: vec![],
                handoff_seq: None,
                handoff_plan: None,
                skill: None,
                task_type: None,
                requirement: None,
            })
            .await
            .unwrap();
        store
            .create_session(&opencoder_store::SessionMeta {
                id: "child-1".into(),
                title: None,
                agent: Some("explore".into()),
                model: Some("m".into()),

                autopilot_mode: None,
                workdir_hash: None,
                created_at: 0,
                updated_at: 0,
                summary: None,
                summary_seq: None,
                summary_images: vec![],
                handoff_seq: None,
                handoff_plan: None,
                skill: None,
                task_type: None,
                requirement: None,
            })
            .await
            .unwrap();
        store
            .create_subagent_task(&SubagentTaskRecord {
                task_id: "call-1".into(),
                parent_session_id: "force-cancel-parent".into(),
                child_session_id: "child-1".into(),
                parent_message_id: None,
                agent: "explore".into(),
                prompt: "x".into(),
                result: None,
                status: SubagentStatus::Running,
                ok: None,
                started_at: 0,
                completed_at: None,
            })
            .await
            .unwrap();

        // Build a session with the store and pre-populated registry entries.
        let mut session = make_session();
        session = session.with_store(store.clone());
        let call_id = "call-1";
        let token = CancellationToken::new();
        session
            .child_cancels
            .lock()
            .unwrap()
            .insert(call_id.to_string(), token);
        session
            .child_turn_cancels
            .lock()
            .unwrap()
            .insert(call_id.to_string(), {
                Arc::new(Mutex::new(CancellationToken::new()))
            });
        session
            .child_steer_gates
            .lock()
            .unwrap()
            .insert(call_id.to_string(), crate::SubagentSteerGate::new());

        let mut noop: Box<dyn FnMut(SessionEvent) + Send> = Box::new(|_| {});
        let sink: Sink<'_> = Arc::new(Mutex::new(&mut *noop));

        force_cancel_subagent(
            session.store.clone(),
            session.child_cancels.clone(),
            session.child_turn_cancels.clone(),
            session.child_steer_gates.clone(),
            &sink,
            call_id,
        )
        .await;

        // DB task must be Cancelled.
        let tasks = store
            .list_subagent_tasks("force-cancel-parent")
            .await
            .unwrap();
        store
            .create_session(&opencoder_store::SessionMeta {
                id: "child-1".into(),
                title: None,
                agent: Some("explore".into()),
                model: Some("m".into()),

                autopilot_mode: None,
                workdir_hash: None,
                created_at: 0,
                updated_at: 0,
                summary: None,
                summary_seq: None,
                summary_images: vec![],
                handoff_seq: None,
                handoff_plan: None,
                skill: None,
                task_type: None,
                requirement: None,
            })
            .await
            .unwrap();
        assert_eq!(tasks.len(), 1);
        assert!(
            matches!(tasks[0].status, SubagentStatus::Cancelled),
            "task must be Cancelled after force_cancel, got {:?}",
            tasks[0].status
        );
        // Registries must be pruned.
        assert!(
            session.child_cancels.lock().unwrap().is_empty(),
            "child_cancels must be empty after force_cancel"
        );
        assert!(
            session.child_turn_cancels.lock().unwrap().is_empty(),
            "child_turn_cancels must be empty after force_cancel"
        );
        assert!(
            session.child_steer_gates.lock().unwrap().is_empty(),
            "child_steer_gates must be empty after force_cancel"
        );
    }
}

#[cfg(test)]
#[path = "execute_timeout_tests.rs"]
mod timeout_tests;
