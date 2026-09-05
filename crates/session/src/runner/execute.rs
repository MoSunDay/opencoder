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
        // Plan admission admits `task` for evidence gathering, but the
        // sidecar kind is read-only-only by contract: gate the spawn here
        // (before any child session is created) so a sidecar cannot farm
        // mutations out to a full write-capable subagent. Same denial text
        // as the generic gate for a consistent UX.
        if let Some(denial) = crate::bash_guard::gate(
            &session.agent.kind,
            &session.agent.name,
            "task",
            None,
            &session.working_dir,
        ) {
            return ToolOutput::err(denial);
        }
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

    // Read-only execution gate (plan mode + sidecar): unadmitted tools AND
    // mutating bash are refused with a model-visible denial (names the
    // session, forbids retry, points at the escape hatch) so the model stops
    // attempting writes instead of looping; see bash_guard::gate for the
    // policy. The effective workdir is
    // resolved exactly like `tools::bash::execute` resolves it (`workdir`
    // input, else the session working dir): the classifier must judge
    // relative writes in the same directory the command will actually run in.
    let effective_workdir = tc
        .input
        .get("workdir")
        .and_then(|v| v.as_str())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| session.working_dir.clone());
    if let Some(denial) = crate::bash_guard::gate(
        &session.agent.kind,
        &session.agent.name,
        &tc.name,
        tc.input.get("command").and_then(|v| v.as_str()),
        &effective_workdir,
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
        // Agent-private tool dirs (file-based agents), colon-joined for the
        // bash tool's PATH prefix. `None` for builtin/plain sessions.
        tools_path: (!session.tools_path.is_empty()).then(|| {
            session
                .tools_path
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(":")
        }),
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
#[path = "execute_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "execute_timeout_tests.rs"]
mod timeout_tests;
