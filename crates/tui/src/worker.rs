//! Background worker command processing — shared by the main worker and the
//! `/task`-spawned worker to avoid duplicate match arms.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use opencoder_core::{message::now_ms, resolve_agent, Config, Role};
use opencoder_llm::ChatClient;
use opencoder_session::{
    control_cmd::persist_agent as persist_session_agent, run as run_session, run_with_images,
    spawn_event_flusher, SessionEvent, SessionState, SharedCancel, SubagentSteerGate,
};
use opencoder_store::{SessionEventRecord, Store};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub enum UiCmd {
    Prompt(String, Vec<String>),
    SwitchAgent(String),
    /// Switch agent then immediately start a turn without recording a new user
    /// message. Used for the plan->act manual transition: the system prompt
    /// changes to act and the model reads the plan from conversation history.
    /// The second field carries any text left in the plan-mode input box; it
    /// is appended to the plan during the handoff so it is submitted too.
    SwitchAndStart(String, String),
    /// Manually trigger conversation compaction.
    Compact,
    SetSkill(Option<String>),
    /// Hot-reload config at the next turn boundary. Sent by the `/config` menu.
    ReloadConfig(Box<Config>),
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
    /// targets the live turn.
    ResetCancel(CancellationToken),
    Quit,
}

#[derive(Debug)]
pub enum UiEvent {
    Session(SessionEvent),
    /// Reliable completed parent answer for repairing TextDelta chunks shed
    /// by the bounded UI channel. Ordered bridge delivery precedes TurnDone.
    AssistantFinal(String),
    TurnDone(String),
}

/// Bounded worker-to-UI channel capacity shared by initial and switched tasks.
pub(crate) const UI_EVENT_CAPACITY: usize = 512;

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

/// Gate for agent-mode switch actions (Shift+Tab / `/act` / `/plan` /
/// `/act_clear_context` / SwitchAgentNoClear). Busy (`running` or a live
/// subagent — callers precompute `running || subagents_running > 0`) means the
/// worker is mid-`run_session`; applying a mode switch then would start the
/// *next* turn with a stale agent while the current model is still answering
/// under the old system prompt — the mode "switch" would complete at an
/// arbitrary partial boundary. Refuse until idle (clean turn boundary). Pure
/// so the running-guard is unit-testable independent of the async event loop.
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

/// Minimum free capacity reserved by the ordered UI forwarder. Parent
/// TextDelta may be shed below this threshold because `AssistantFinal` repairs
/// it. Every other event is delivered with async backpressure in original
/// order, including child deltas, reasoning, transcript resets and lifecycle.
const DELTA_MIN_CAPACITY: usize = 64;

/// Returns true for parent streaming text whose completed value is repaired by
/// the reliable `AssistantFinal` event. Child deltas are not recoverable here.
/// ReasoningDelta is deliberately excluded — see `DELTA_MIN_CAPACITY` docs.
fn is_droppable_delta(sev: &SessionEvent) -> bool {
    matches!(sev, SessionEvent::TextDelta(_))
}

/// Enqueue a session event into the per-command ordered bridge. The sync LLM
/// callback never writes directly to the bounded UI channel; the bridge task
/// owns that operation so reliable events can await capacity without blocking
/// or reordering the callback.
fn forward_event(tx: &mpsc::UnboundedSender<UiEvent>, sev: SessionEvent) {
    let _ = tx.send(UiEvent::Session(sev));
}

fn spawn_ui_event_forwarder(
    tx: mpsc::Sender<UiEvent>,
) -> (mpsc::UnboundedSender<UiEvent>, tokio::task::JoinHandle<()>) {
    let (pending_tx, mut pending_rx) = mpsc::unbounded_channel::<UiEvent>();
    let handle = tokio::spawn(async move {
        while let Some(event) = pending_rx.recv().await {
            let droppable = matches!(&event, UiEvent::Session(sev) if is_droppable_delta(sev));
            if droppable && tx.capacity() <= DELTA_MIN_CAPACITY {
                continue;
            }
            if tx.send(event).await.is_err() {
                break;
            }
        }
    });
    (pending_tx, handle)
}

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
/// immediate exit. Used by non-run arms (e.g. SwitchAgent) where no flusher
/// is active.
async fn persist_event(store: &Option<Arc<dyn Store>>, session_id: &str, sev: &SessionEvent) {
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
) -> bool {
    let (ui_tx, ui_forwarder) = spawn_ui_event_forwarder(evt_tx.clone());
    let quit = match cmd {
        UiCmd::Prompt(prompt, images) => {
            let message_floor = sess.messages.len();
            let tx = ui_tx.clone();
            let (sink, flusher) = spawn_event_flusher(sess.store.clone(), sess.id.clone());
            let sink_for_run = sink.clone();
            let res = if images.is_empty() {
                run_session(sess, prompt, move |sev| {
                    let _ = sink_for_run.push(&sev);
                    forward_event(&tx, sev);
                })
                .await
            } else {
                run_with_images(sess, prompt, images, move |sev| {
                    let _ = sink_for_run.push(&sev);
                    forward_event(&tx, sev);
                })
                .await
            };
            if let Err(e) = res {
                let ev = SessionEvent::Error(format!("{e:#}"));
                let _ = sink.push(&ev);
                forward_event(&ui_tx, ev);
            }
            // Drop every sender clone so the flusher's channel closes and it
            // performs a final flush — guaranteeing zero event loss this turn.
            drop(sink);
            let _ = flusher.await;
            send_completed_assistant(&ui_tx, sess, message_floor);
            let _ = ui_tx.send(UiEvent::TurnDone(sess.agent.name.clone()));
            false
        }
        UiCmd::SwitchAgent(name) => {
            // DEFENSE-IN-DEPTH: this arm is only reachable at a clean turn
            // boundary. The worker loop is single-threaded and `run_session`
            // is synchronous within `process_cmd(UiCmd::Prompt)` — a switch
            // queued during a live turn is not consumed until that `process_cmd`
            // returns, so `sess.agent` is never flipped mid-`run_session`.
            // The app-loop running-gate (`gate_switch` / `handle_switch_agent`)
            // additionally refuses to SEND a switch while `running` is true.
            if let Some(a) = resolve_agent(&name) {
                sess.agent = a;
                // Mirror control_cmd::apply: switching to plan resets the
                // plan-input counter so the "submit your plan" reminder logic
                // starts from a fresh phase. Without this the TUI key-handler
                // path (Alt+Tab / Ctrl+T) inherited a stale nonzero count,
                // unlike the `/plan` slash-command path.
                if name == "plan" {
                    sess.plan_input_count = 0;
                }
                let ev = SessionEvent::AgentSwitch(name.clone());
                persist_event(&sess.store, &sess.id, &ev).await;
                forward_event(&ui_tx, ev);
                if let Err(e) = persist_session_agent(sess, &name).await {
                    tracing::warn!(error = %e, "persist_session_agent failed");
                }
            }
            false
        }
        UiCmd::SwitchAndStart(name, extra) => {
            let (sink, flusher) = spawn_event_flusher(sess.store.clone(), sess.id.clone());
            if let Some(a) = resolve_agent(&name) {
                sess.agent = a;
                let ev = SessionEvent::AgentSwitch(name.clone());
                let _ = sink.push(&ev);
                forward_event(&ui_tx, ev);
                if let Err(e) = persist_session_agent(sess, &name).await {
                    tracing::warn!(error = %e, "persist_session_agent failed");
                }
            }
            // Plan→act handoff: clear the transcript so the act agent starts
            // from only the final plan, not the full read-only planning noise.
            // Mirrors compaction — in-memory mutation + TranscriptReset so the
            // UI rebuilds clean; the append-only store keeps the raw history.
            if let Some(plan_display) = opencoder_session::plan_handoff::handoff(sess, &extra) {
                // Persist the handoff boundary so resume reconstructs the
                // focused post-handoff transcript (mirrors compaction).
                if let Some(store) = &sess.store {
                    let _ = store
                        .update_session(
                            &sess.id,
                            &opencoder_store::SessionPatch {
                                handoff_seq: sess.handoff_seq,
                                handoff_plan: sess.handoff_plan.clone(),
                                clear_skill: true,
                                updated_at: Some(now_ms()),
                                ..Default::default()
                            },
                        )
                        .await;
                }
                let ev = SessionEvent::TranscriptReset(sess.messages.clone());
                let _ = sink.push(&ev);
                forward_event(&ui_tx, ev);
                let ev2 = SessionEvent::PlanHandoff(plan_display);
                let _ = sink.push(&ev2);
                forward_event(&ui_tx, ev2);
            }
            sess.set_skill(None);
            let message_floor = sess.messages.len();
            let tx = ui_tx.clone();
            let sink_for_run = sink.clone();
            let res = run_session(sess, String::new(), move |sev| {
                let _ = sink_for_run.push(&sev);
                forward_event(&tx, sev);
            })
            .await;
            if let Err(e) = res {
                let ev = SessionEvent::Error(format!("{e:#}"));
                let _ = sink.push(&ev);
                forward_event(&ui_tx, ev);
            }
            drop(sink);
            let _ = flusher.await;
            send_completed_assistant(&ui_tx, sess, message_floor);
            let _ = ui_tx.send(UiEvent::TurnDone(sess.agent.name.clone()));
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
                }
                Ok(None) => {}
                Err(e) => {
                    let ev = SessionEvent::Error(format!("compaction failed: {e:#}"));
                    let _ = sink.push(&ev);
                    forward_event(&ui_tx, ev);
                }
            }
            drop(sink);
            let _ = flusher.await;
            let _ = ui_tx.send(UiEvent::TurnDone(sess.agent.name.clone()));
            false
        }
        UiCmd::SetSkill(body) => {
            sess.set_skill(body);
            false
        }
        UiCmd::ReloadConfig(new_cfg) => {
            let applied_model;
            let prev_model = sess.config.model.clone();
            match new_cfg.resolve_endpoint() {
                Ok(ep) => match ChatClient::new_with_read_timeout(
                    &ep.base_url,
                    &ep.api_key,
                    &ep.headers,
                    new_cfg.stream_idle_timeout(),
                    new_cfg.network.proxy.as_deref(),
                ) {
                    Ok(new_client) => {
                        sess.apply_config_reload(*new_cfg, Arc::new(new_client));
                        applied_model = true;
                    }
                    Err(e) => {
                        let model = new_cfg.model_id().to_string();
                        sess.apply_config_reload_keep_client(*new_cfg);
                        let msg = format!(
                            "model switched to {model} but client build failed \
                             ({e:#}); keeping previous client"
                        );
                        let ev = SessionEvent::Error(msg);
                        forward_event(&ui_tx, ev);
                        applied_model = true;
                    }
                },
                Err(e) => {
                    let model = new_cfg.model_id().to_string();
                    sess.apply_config_reload_keep_client(*new_cfg);
                    let msg = format!(
                        "model switched to {model} but endpoint resolve failed \
                         ({e:#}); keeping previous client"
                    );
                    let ev = SessionEvent::Error(msg);
                    forward_event(&ui_tx, ev);
                    applied_model = true;
                }
            }
            // Persist the switched model to the store so resume() honors it
            // (otherwise the stale `sessions.model` column reverts the switch
            // on the next /task resume or `opencode -s <id>` restart). Only
            // when the model string actually changed: `/ap` and pure
            // max_iterations saves also land here, and must not surface a
            // spurious `[model]` marker or rewrite the store column.
            if applied_model && sess.config.model != prev_model {
                // The store column keeps the full `provider/model` string
                // (resume honors it); the ModelSwitch display marker uses the
                // bare model id so it matches the status bar (issue #1).
                let model_full = sess.config.model.clone();
                if let Some(store) = &sess.store {
                    let _ = store
                        .update_session(
                            &sess.id,
                            &opencoder_store::SessionPatch {
                                model: Some(model_full),
                                updated_at: Some(now_ms()),
                                ..Default::default()
                            },
                        )
                        .await;
                }
                let ev = SessionEvent::ModelSwitch(sess.config.model_id().to_string());
                persist_event(&sess.store, &sess.id, &ev).await;
                forward_event(&ui_tx, ev);
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
            sess.requirement = Some(text.clone());
            if let Some(store) = &sess.store {
                let _ = store
                    .update_session(
                        &sess.id,
                        &opencoder_store::SessionPatch {
                            requirement: Some(text),
                            ..Default::default()
                        },
                    )
                    .await;
            }
            false
        }
        UiCmd::ResetCancel(c) => {
            sess.cancel = Some(c);
            false
        }
        UiCmd::Quit => true,
    };
    drop(ui_tx);
    let _ = ui_forwarder.await;
    quit
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_reload;
