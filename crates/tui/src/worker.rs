//! Background worker command processing — shared by the main worker and the
//! `/task`-spawned worker to avoid duplicate match arms.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use opencoder_core::{message::now_ms, Config, Role};
use opencoder_session::{
    run as run_session, run_with_images, spawn_event_flusher, SessionEvent, SessionState,
    SharedCancel, SubagentSteerGate,
};
use opencoder_store::{SessionEventRecord, Store};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[path = "worker/model_commands.rs"]
mod model_commands;

/// One-shot, operation-local completion channel. Each request owns a distinct
/// channel, so late or unrelated session errors cannot complete/roll it back.
pub type ConfigAck = mpsc::UnboundedSender<Result<(), String>>;

/// Which part of a runtime configuration a reload is allowed to replace.
/// Domain reloads deliberately retain the worker's Provider/model/mode state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigReloadScope {
    Full,
    Cli,
    Skills,
}

/// Build an operation-correlated reload command. The UI must wait for the
/// completion before committing its corresponding outer runtime state.
pub(crate) fn acknowledged_config_reload(
    config: Config,
    scope: ConfigReloadScope,
) -> (UiCmd, mpsc::UnboundedReceiver<Result<(), String>>) {
    let (ack, completion) = mpsc::unbounded_channel();
    (
        UiCmd::ReloadConfig {
            config: Box::new(config),
            scope,
            ack: Some(ack),
        },
        completion,
    )
}

/// `Clone` keeps the turn-starting commands cheap to forward through the
/// single-threaded worker command channel.
#[derive(Debug, Clone)]
pub enum UiCmd {
    Prompt(String, Vec<String>),
    /// Manually trigger conversation compaction.
    Compact,
    SetSkill(Option<String>),
    /// Hot-reload config at the next turn boundary. Full reloads persist a
    /// changed Shared model before committing worker memory. Cli/Skills
    /// reloads replace only that domain and never rebuild the model client.
    ReloadConfig {
        config: Box<Config>,
        scope: ConfigReloadScope,
        /// Legacy fire-and-forget callers may omit this; interactive model and
        /// domain saves use a correlated acknowledgment.
        ack: Option<ConfigAck>,
    },
    /// Apply a session-scoped Provider/model patch and confirm its commit.
    /// `provider_changed` alone controls client reconstruction.
    ApplySession {
        config: Box<Config>,
        selection: Option<(opencoder_core::ConfigMode, String)>,
        provider_changed: bool,
        ack: ConfigAck,
    },
    /// Reload a freshly selected local Env and clear durable Act/Plan overrides.
    /// The UI commits its outer state only after this operation acknowledges.
    ReloadEnv {
        config: Box<Config>,
        ack: ConfigAck,
    },
    /// Replace the plan text in the last non-empty Assistant message in-memory.
    /// Does not touch the append-only store (consistent with compaction/handoff
    /// which also rewrite the in-memory `messages` without appending a record).
    /// On resume the original (un-edited) plan is reloaded from the store.
    EditPlan(String),
    /// Replace the annotation text on the session and persist it to the
    /// store (unlike EditPlan which is in-memory only).
    EditAnnotation(String),
    /// Swap the session's cancellation token for a fresh, uncancelled one.
    /// Sent before every turn-starting command so a prior double-Esc abort
    /// doesn't leave `sess.cancel` permanently cancelled (which would make
    /// `run_loop` break instantly at its top-of-loop `is_cancelled()` check,
    /// silently rejecting all subsequent submissions). The loop reassigns its
    /// own `cancel` handle to a clone of the same token so double-Esc still
    /// targets the live turn. The `u64` is the TUI's turn generation: echoed
    /// back on the matching `TurnDone` so stale completions from a cancelled
    /// turn are dropped by the UI.
    ResetCancel(CancellationToken, u64),
    /// Force-interrupt the remote server-side turn so a pending steer is
    /// absorbed at its boundary (queue-panel `>` on a remote session). The
    /// local worker ignores it: local `>` already interrupts through the
    /// shared `turn_cancel` token, which — unlike this command — is observed
    /// mid-turn.
    Interrupt,
    Quit,
}

#[derive(Debug)]
pub enum UiEvent {
    RemoteSnapshot {
        chat: Box<crate::chat::ChatView>,
        running: bool,
    },
    Session(SessionEvent),
    /// Authoritative completed parent answer. Ordered bridge delivery precedes
    /// TurnDone; every interim streaming answer is also delivered losslessly.
    AssistantFinal(String),
    /// The `u64` echoes the turn generation from the `ResetCancel` that armed
    /// the turn; the UI drops `TurnDone`s whose generation no longer matches.
    TurnDone(String, u64),
    /// Background subtask watcher progress. `session` is the parent session whose
    /// registry owns the subtask; delivered on a TUI-internal channel so it
    /// survives session switches (workers are quit on switch).
    SubtaskUpdate {
        session: String,
        id: String,
        status: String,
        summary: Option<String>,
        tokens: u64,
    },
}

/// Bounded worker-to-UI channel capacity shared by initial and switched tasks.
pub(crate) const UI_EVENT_CAPACITY: usize = 512;

pub(crate) fn spawn_task(
    mut session: SessionState,
    mut commands: mpsc::Receiver<UiCmd>,
    ui: mpsc::Sender<UiEvent>,
) -> tokio::task::JoinHandle<()> {
    session.harness.literal_mentions = true;
    if session.harness.remote.is_some() {
        return crate::remote::spawn(session, commands, ui);
    }
    tokio::spawn(async move {
        let mut turn = 0u64;
        while let Some(command) = commands.recv().await {
            if let UiCmd::ResetCancel(_, next) = &command {
                turn = *next;
            }
            if process_cmd(command, &mut session, &ui, turn).await {
                break;
            }
        }
    })
}

/// Session-scoped child runtime registries used by TUI controls while the
/// worker owns the corresponding [`SessionState`]. These handles must move as
/// one unit on `/task` switches; retaining any registry from the previous
/// session makes child steer/cancel actions target stale runners.
#[derive(Clone)]
pub struct ChildRuntimeHandles {
    pub cancels: Arc<Mutex<HashMap<String, CancellationToken>>>,
    pub turn_cancels: Arc<Mutex<HashMap<String, SharedCancel>>>,
    pub steer_gates: Arc<Mutex<HashMap<String, Arc<SubagentSteerGate>>>>,
}

impl ChildRuntimeHandles {
    pub fn from_session(session: &SessionState) -> Self {
        Self {
            cancels: session.child_cancels.clone(),
            turn_cancels: session.child_turn_cancels.clone(),
            steer_gates: session.child_steer_gates.clone(),
        }
    }
}

/// Rebind the main loop's session-scoped handles to a freshly switched session.
///
/// Called after `/task` picks a new/resumed session. Channels, parent cancel
/// tokens and all child registries move together. Retaining any handle from the
/// first session makes switched sessions partially uninterruptible or rejects
/// valid child steers against the stale admission-gate map.
#[allow(clippy::too_many_arguments)]
pub fn rebind_session(
    cmd_tx: &mut mpsc::Sender<UiCmd>,
    evt_rx: &mut mpsc::Receiver<UiEvent>,
    session_id: &mut String,
    cancel: &mut CancellationToken,
    turn_cancel: &mut SharedCancel,
    child_runtime: &mut ChildRuntimeHandles,
    new_cmd_tx: mpsc::Sender<UiCmd>,
    new_evt_rx: mpsc::Receiver<UiEvent>,
    new_session_id: String,
    new_cancel: CancellationToken,
    new_turn_cancel: SharedCancel,
    new_child_runtime: ChildRuntimeHandles,
) {
    *cmd_tx = new_cmd_tx;
    *evt_rx = new_evt_rx;
    *session_id = new_session_id;
    *cancel = new_cancel;
    *turn_cancel = new_turn_cancel;
    *child_runtime = new_child_runtime;
}

/// `/compact` dispatch policy: only run when idle. Kept as a pure function so
/// the running-guard (and its busy feedback) is unit-testable independent of the
/// async event loop.
#[derive(Debug, PartialEq, Eq)]
pub enum CompactGate {
    Run,
    SkipRunning,
}

pub fn gate_compact(running: bool) -> CompactGate {
    if running {
        CompactGate::SkipRunning
    } else {
        CompactGate::Run
    }
}

/// Gate for the `/task` "Clear all" destructive action. A turn in flight
/// (`running == true`) means a subagent may still be writing to its child
/// session — clearing then would yank that row out from under it (FK
/// violation on the next append). Refuse until idle (all subagents returned).
#[derive(Debug, PartialEq, Eq)]
pub enum ClearAllGate {
    Run,
    SkipRunning,
}

pub fn gate_clear_all(running: bool) -> ClearAllGate {
    if running {
        ClearAllGate::SkipRunning
    } else {
        ClearAllGate::Run
    }
}

/// Gate for control-command dispatch (`/act`, `/plan`,
/// `/act_clear_context` — and Shift+Tab, which arms the countdown guard from
/// plan mode and switches straight back to plan from act mode).
/// Busy (`running` — the caller passes the parent session's state; a live
/// subagent does NOT count: the parent is idle, exactly when steer/queue
/// entries are consumed automatically) means the worker is mid-
/// `run_session`; starting a control-command turn then would race the
/// in-flight turn at an arbitrary partial boundary.
/// Pure so the running-guard is unit-testable independent of the async event
/// loop.
///
/// While busy the dispatcher REFUSES the switch with the shared busy flash
/// (`mode_switch_busy_flash`) instead of queueing it: a mid-turn switch is
/// never applied and never deferred — the user retries when idle.
#[derive(Debug, PartialEq, Eq)]
pub enum SwitchGate {
    Run,
    SkipRunning,
}

pub fn gate_switch(busy: bool) -> SwitchGate {
    if busy {
        SwitchGate::SkipRunning
    } else {
        SwitchGate::Run
    }
}

#[path = "worker/delivery.rs"]
mod delivery;
use delivery::{forward_event, spawn_ui_event_forwarder};

#[cfg(test)]
#[path = "worker/delivery_tests.rs"]
mod delivery_tests;

fn completed_assistant_text(sess: &SessionState, message_floor: usize) -> Option<String> {
    sess.messages
        .get(message_floor..)?
        .iter()
        .rev()
        .find(|message| message.role == Role::Assistant && !message.text().is_empty())
        .map(|message| message.text())
}

fn send_completed_assistant(
    tx: &mpsc::UnboundedSender<UiEvent>,
    sess: &SessionState,
    message_floor: usize,
) {
    if let Some(text) = completed_assistant_text(sess, message_floor) {
        let _ = tx.send(UiEvent::AssistantFinal(text));
    }
}

/// Fire-and-forget persist a parent-session event to the store so web/SSE
/// clients can replay sessions driven by the TUI. Awaited (not fire-and-
/// forget) so the event is durable before the worker proceeds — no loss on
/// immediate exit. Used by non-run arms where no flusher
/// is active. `pub(crate)` so the sidecar actor (`sidecar_ui`) reuses the
/// exact same persistence shape for the bare `LlmUsage` records it forwards.
pub(crate) async fn persist_event(
    store: &Option<Arc<dyn Store>>,
    session_id: &str,
    sev: &SessionEvent,
) {
    // Sidecar frames are display-only: the sidecar conversation has no
    // session/message rows and its content must never land in the main
    // session's event log. Its cost is accounted to the main task through
    // the *bare* `LlmUsage` events the child forwards, which DO persist.
    if sev.is_sidecar_frame() {
        return;
    }
    if let Some(store) = store {
        let rec = SessionEventRecord {
            session_id: session_id.to_string(),
            kind: sev.coarse_kind(),
            payload: sev.sse_data(),
            ts: now_ms(),
            seq: None,
            sse_kind: Some(sev.sse_kind().to_string()),
        };
        let _ = store.append_event(&rec).await;
    }
}

/// Process one UI command against a session. Returns `true` when the worker
/// loop should break (Quit).
pub async fn process_cmd(
    cmd: UiCmd,
    sess: &mut SessionState,
    evt_tx: &mpsc::Sender<UiEvent>,
    turn: u64,
) -> bool {
    sess.harness.literal_mentions = true;
    let (ui_tx, ui_forwarder) = spawn_ui_event_forwarder(evt_tx.clone());
    let quit = match cmd {
        UiCmd::Prompt(prompt, images) => {
            // Repair floor for the reliable `AssistantFinal`: the message
            // count at run start, EXCEPT that a mid-run compaction
            // (`TranscriptReset`) replaces the whole message list with a
            // short summary, shifting every index below the stale floor —
            // the completed answer would then be invisible to
            // `completed_assistant_text` and the shed-delta repair silently
            // lost. Track the reset so the floor follows the new list.
            let message_floor = std::sync::atomic::AtomicUsize::new(sess.messages.len());
            // The runner reports fatal failures as a `SessionEvent::Error`
            // before returning `Err`; remember that so the fallback below only
            // fires for failures that never produced an error event. Otherwise
            // every failed turn is shown (and persisted) twice.
            let run_emitted_error = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let (sink, flusher) = spawn_event_flusher(sess.store.clone(), sess.id.clone());
            let sink_for_run = sink.clone();
            let res = if images.is_empty() {
                let tx = ui_tx.clone();
                let floor = &message_floor;
                let emitted = Arc::clone(&run_emitted_error);
                run_session(sess, prompt, move |sev| {
                    if let SessionEvent::TranscriptReset(msgs) = &sev {
                        floor.store(msgs.len(), std::sync::atomic::Ordering::Relaxed);
                    }
                    if matches!(sev, SessionEvent::Error(_)) {
                        emitted.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    let _ = sink_for_run.push(&sev);
                    forward_event(&tx, sev);
                })
                .await
            } else {
                let tx = ui_tx.clone();
                let floor = &message_floor;
                let emitted = Arc::clone(&run_emitted_error);
                run_with_images(sess, prompt, images, move |sev| {
                    if let SessionEvent::TranscriptReset(msgs) = &sev {
                        floor.store(msgs.len(), std::sync::atomic::Ordering::Relaxed);
                    }
                    if matches!(sev, SessionEvent::Error(_)) {
                        emitted.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    let _ = sink_for_run.push(&sev);
                    forward_event(&tx, sev);
                })
                .await
            };
            if let Err(e) = res {
                if !run_emitted_error.load(std::sync::atomic::Ordering::Relaxed) {
                    let ev = SessionEvent::Error(format!("{e:#}"));
                    let _ = sink.push(&ev);
                    forward_event(&ui_tx, ev);
                }
            }
            // Drop every sender clone so the flusher's channel closes and it
            // performs a final flush — guaranteeing zero event loss this turn.
            drop(sink);
            let _ = flusher.await;
            send_completed_assistant(
                &ui_tx,
                sess,
                message_floor.load(std::sync::atomic::Ordering::Relaxed),
            );
            let _ = ui_tx.send(UiEvent::TurnDone(sess.agent.name.clone(), turn));
            false
        }
        UiCmd::Compact => {
            let registry = opencoder_session::tools::registry();
            let (sink, flusher) = spawn_event_flusher(sess.store.clone(), sess.id.clone());
            // Scope the emit closure so its sender clone is dropped before we
            // drop the last sender + await the flusher (final flush).
            let outcome = {
                let tx = ui_tx.clone();
                let sink_for_emit = sink.clone();
                let mut emit = move |sev: SessionEvent| {
                    let _ = sink_for_emit.push(&sev);
                    forward_event(&tx, sev);
                };
                opencoder_session::compaction::compact(sess, &registry, &mut emit).await
            };
            match outcome {
                Ok(Some(summary)) => {
                    let ev = SessionEvent::TranscriptReset(sess.messages.clone());
                    let _ = sink.push(&ev);
                    forward_event(&ui_tx, ev);
                    let ev2 = SessionEvent::Compaction(summary);
                    let _ = sink.push(&ev2);
                    forward_event(&ui_tx, ev2);
                    // Web parity (handle.rs DrainCmd::Compact): a successful
                    // compact is a completed drain command, so it must end
                    // with a terminal Done frame. The app_loop Done handler
                    // re-syncs pending Queue/Steer rows from the store and
                    // arms `drain_pending`; without Done, inputs admitted
                    // while the compaction turn ran strand in the store
                    // forever (TurnDone alone never resyncs).
                    let ev3 = SessionEvent::Done;
                    let _ = sink.push(&ev3);
                    forward_event(&ui_tx, ev3);
                }
                Ok(None) => {
                    // "Nothing to compact yet" is still a successful command:
                    // web emits Done for every `Ok(_)` outcome, so the idle
                    // boundary stays consistent across both frontends.
                    let ev = SessionEvent::Done;
                    let _ = sink.push(&ev);
                    forward_event(&ui_tx, ev);
                }
                Err(e) => {
                    let ev = SessionEvent::Error(format!("compaction failed: {e:#}"));
                    let _ = sink.push(&ev);
                    forward_event(&ui_tx, ev);
                }
            }
            drop(sink);
            let _ = flusher.await;
            let _ = ui_tx.send(UiEvent::TurnDone(sess.agent.name.clone(), turn));
            false
        }
        UiCmd::SetSkill(body) => {
            sess.set_skill(body);
            false
        }
        UiCmd::ApplySession {
            config,
            selection,
            provider_changed,
            ack,
        } => {
            let result =
                model_commands::apply_session(sess, &ui_tx, *config, selection, provider_changed)
                    .await;
            let _ = ack.send(result);
            false
        }
        UiCmd::ReloadEnv { config, ack } => {
            let result = model_commands::reload_env(sess, &ui_tx, *config).await;
            let _ = ack.send(result);
            false
        }
        UiCmd::ReloadConfig { config, scope, ack } => {
            let result = model_commands::reload_config(sess, &ui_tx, *config, scope).await;
            if let Some(ack) = ack {
                let _ = ack.send(result);
            } else if let Err(error) = result {
                // Fire-and-forget callers have no correlated UI receiver. Keep
                // the failure visible without pretending the reload succeeded.
                forward_event(&ui_tx, SessionEvent::Error(error));
            }
            false
        }
        UiCmd::EditPlan(new_text) => {
            // Find the last Assistant message whose `text()` is non-empty and
            // replace its Text blocks with a single block carrying the edited
            // text. Non-Text blocks (Reasoning, ToolUse, etc.) are preserved.
            for msg in sess.messages.iter_mut().rev() {
                if msg.role != opencoder_core::Role::Assistant {
                    continue;
                }
                if msg.text().trim().is_empty() {
                    continue;
                }
                let mut new_blocks: Vec<opencoder_core::ContentBlock> = msg
                    .blocks
                    .iter()
                    .filter(|b| !matches!(b, opencoder_core::ContentBlock::Text { .. }))
                    .cloned()
                    .collect();
                new_blocks.push(opencoder_core::ContentBlock::Text {
                    text: new_text.clone(),
                });
                msg.blocks = new_blocks;
                break;
            }
            false
        }
        UiCmd::EditAnnotation(text) => {
            if text.trim().is_empty() {
                // Blank submit is an explicit clear: drop the requirement
                // in-memory and persist the clear.
                sess.requirement = None;
                if let Some(store) = &sess.store {
                    if let Err(e) = store
                        .update_session(
                            &sess.id,
                            &opencoder_store::SessionPatch {
                                clear_requirement: true,
                                ..Default::default()
                            },
                        )
                        .await
                    {
                        tracing::warn!(error = %e, "persist requirement clear failed");
                    }
                }
            } else {
                sess.requirement = Some(text.clone());
                if let Some(store) = &sess.store {
                    if let Err(e) = store
                        .update_session(
                            &sess.id,
                            &opencoder_store::SessionPatch {
                                requirement: Some(text),
                                ..Default::default()
                            },
                        )
                        .await
                    {
                        tracing::warn!(error = %e, "persist requirement failed");
                    }
                }
            }
            false
        }
        UiCmd::ResetCancel(c, _) => {
            sess.cancel = Some(c);
            false
        }
        // No-op locally: local `>` interrupts via the shared turn_cancel
        // token, which (unlike this command) is observed mid-turn.
        UiCmd::Interrupt => false,
        UiCmd::Quit => true,
    };
    drop(ui_tx);
    let _ = ui_forwarder.await;
    quit
}

#[cfg(test)]
mod tests;
#[cfg(test)]
#[path = "worker/tests_compact_done.rs"]
mod tests_compact_done;
#[cfg(test)]
mod tests_failures;
#[cfg(test)]
mod tests_model_commands;
#[cfg(test)]
mod tests_reload;
#[cfg(test)]
#[path = "worker/tests_sidecar.rs"]
mod tests_sidecar;
