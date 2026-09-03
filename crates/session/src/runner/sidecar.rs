//! Sidecar: a temporary agent loop that answers questions about the main
//! session's context snapshot (TUI `/sidecar <question>`).
//!
//! Contract:
//! - **Snapshot-in**: the child `SessionState` starts from a clone of the
//!   parent transcript (or a caller-supplied snapshot), then keeps its own
//!   in-memory Q/A history so follow-up turns see prior turns.
//! - **Zero persistence**: the child is never `.with_store()`-attached, so
//!   `record`/`persist` are no-ops - no session row, no message rows. Sidecar
//!   content frames (`Sidecar*` events) are display-only and dropped by
//!   `EventSink::push` (`SessionEvent::is_sidecar_frame`).
//! - **Cost lands on the main task**: every child `LlmUsage` is forwarded as
//!   a *bare* event (not wrapped in `SidecarChild`) so downstream surfaces
//!   accumulate and persist it exactly like a main-task round.
//!
//! Free functions + plain structs only; conversation state is carried
//! explicitly in [`SidecarConv`].

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{anyhow, Result};
use opencoder_core::{resolve_agent, ApMode, Config, Message, Role, ToolArc};
use opencoder_llm::ChatStream;

use super::event::Sink;
use super::registry::build_full_registry;
use super::{emit, new_id, run_with_registry, SessionEvent};
use crate::{control_cmd, SessionState};

/// Session-id prefix for sidecar conversations (mirrors `sub-` for task
/// subagents). Keeps accidental store inspection unambiguous.
pub const SIDECAR_ID_PREFIX: &str = "sidecar-";

/// One sidecar conversation: a store-less child session seeded with the main
/// session's context snapshot, plus the tool registry built for it. Reuse the
/// same value for follow-up questions - the child's message list continues
/// the Q/A history in memory.
pub struct SidecarConv {
    /// `"sidecar-<ulid>"`; tags every forwarded frame.
    pub id: String,
    /// The temporary loop. No store attached: nothing it records persists.
    pub child: SessionState,
    /// Full tool registry for the child (schema filtering by the sidecar
    /// agent's read-only `ToolFilter` happens at request build time).
    pub registry: HashMap<String, ToolArc>,
}

/// Result summary of one sidecar question. Mirrors the display-only
/// [`SessionEvent::SidecarTurn`] frame.
pub struct SidecarTurn {
    pub ok: bool,
    /// Final assistant text of this turn (the captured error message when
    /// the loop failed and produced no text).
    pub answer: String,
    pub elapsed_ms: u64,
    /// Token cost of this turn (already forwarded bare to the parent's
    /// event stream for main-task accounting).
    pub total_tokens: u64,
    /// LLM rounds consumed by this turn (usage-event count).
    pub rounds: usize,
}

/// Per-turn accumulator shared with the forwarding closure (`FnMut` cannot
/// hold several mutable borrows; the mutex also keeps the closure `Send`).
#[derive(Default, Clone)]
struct TurnAcc {
    total_tokens: u64,
    rounds: usize,
    error: Option<String>,
}

/// Recognize the `/sidecar` TUI-local command. Returns the trimmed question
/// (possibly empty for a bare `/sidecar`). Anything that is not exactly this
/// token - including look-alikes like `/sidecarX` - is `None` so the text
/// flows to the normal prompt path.
pub fn parse_sidecar_question(text: &str) -> Option<String> {
    let rest = text.trim().strip_prefix("/sidecar")?;
    // Word boundary: `/sidecarX` is a different command, not ours.
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    Some(rest.trim().to_string())
}

/// Read-only clone of the parent transcript: the sidecar's background
/// context. Cloned (not moved) so the parent view is untouched.
pub fn snapshot_messages(parent: &SessionState) -> Vec<Message> {
    parent.messages.clone()
}

/// Shared constructor for both entry points: a store-less child with the
/// sidecar agent, autopilot forced off, and the snapshot as its transcript.
async fn build_sidecar_conv(
    id: String,
    config: Config,
    client: Arc<dyn ChatStream>,
    working_dir: PathBuf,
    snapshot: Vec<Message>,
) -> Result<SidecarConv> {
    let agent =
        resolve_agent("sidecar").ok_or_else(|| anyhow!("'sidecar' agent not registered"))?;
    let mut child = SessionState::new(id.clone(), agent, config, client, working_dir);
    // Autopilot is top-level orchestration only (same rationale as the task
    // subagent): a sidecar answers and stops, never drives its own
    // PLAN->ACT->VERIFY loop after the scoped question.
    child.config.autopilot.mode = ApMode::Off;
    // The transcript is a borrowed snapshot of the parent, not this loop's own
    // durable history: never compact. Without this, a parent near its compaction
    // threshold would make every sidecar question first pay a compaction LLM
    // round (and replace the snapshot with a summary) before answering.
    child.config.compaction.auto = false;
    // Deliberately NOT `.with_store()`: record/persist become no-ops, so the
    // sidecar leaves zero rows in the store (zero-persistence loop contract).
    child.messages = snapshot;
    let registry = build_full_registry(&child).await;
    Ok(SidecarConv {
        id,
        child,
        registry,
    })
}

/// Snapshot from a live parent session (runner-side path).
pub async fn new_conv(parent: &SessionState) -> Result<SidecarConv> {
    build_sidecar_conv(
        format!("{SIDECAR_ID_PREFIX}{}", new_id()),
        parent.config.clone(),
        parent.client.clone(),
        parent.working_dir.clone(),
        snapshot_messages(parent),
    )
    .await
}

/// Snapshot supplied by the caller (TUI Phase 2 path): the frontend reads the
/// transcript from the store itself, so no parent `SessionState` is needed.
pub async fn new_conv_from(
    config: Config,
    client: Arc<dyn ChatStream>,
    working_dir: PathBuf,
    snapshot: Vec<Message>,
) -> Result<SidecarConv> {
    build_sidecar_conv(
        format!("{SIDECAR_ID_PREFIX}{}", new_id()),
        config,
        client,
        working_dir,
        snapshot,
    )
    .await
}

/// Run one sidecar question through the temporary loop. Follow-ups: call
/// again on the same [`SidecarConv`] - the child's message history (snapshot
/// + all prior Q/A) continues, so the model sees the earlier turns.
///
/// Events reaching `on_event`:
/// - `LlmUsage` - forwarded **bare** (parent-task cost accounting);
/// - `SidecarChild` - every other child frame (deltas, tool calls, status);
/// - `SidecarTurn` - this turn's summary frame;
/// - child `Done`/`Error` - swallowed: the turn boundary and the failure are
///   expressed by `SidecarTurn`, and a bare `Error` would corrupt the
///   parent's UI state.
pub async fn run_sidecar_turn(
    conv: &mut SidecarConv,
    question: &str,
    on_event: &mut (dyn FnMut(SessionEvent) + Send),
) -> Result<SidecarTurn> {
    // Control commands are parent-session navigation (`/act`, `/plan`,
    // `/act_clear_context`), not questions. Running one here would mutate the
    // child session and desync it from the main session it observes.
    if control_cmd::split_control_prefix(question).is_some() {
        let answer = "sidecar question must not be a control command".to_string();
        let turn = SidecarTurn {
            ok: false,
            answer: answer.clone(),
            elapsed_ms: 0,
            total_tokens: 0,
            rounds: 0,
        };
        let reject_sink: Sink<'_> = Arc::new(Mutex::new(&mut *on_event));
        emit(
            &reject_sink,
            SessionEvent::SidecarTurn {
                id: conv.id.clone(),
                ok: false,
                answer,
                elapsed_ms: 0,
                total_tokens: 0,
                rounds: 0,
            },
        );
        return Ok(turn);
    }

    let started = Instant::now();
    // Only assistant text produced by THIS turn counts as the answer: the
    // snapshot tail may itself end with an old parent assistant message, and
    // returning that would read as a fresh answer.
    let baseline = conv.child.messages.len();

    let acc: Arc<Mutex<TurnAcc>> = Arc::new(Mutex::new(TurnAcc::default()));
    let turn_acc = Arc::clone(&acc);
    let sink: Sink<'_> = Arc::new(Mutex::new(on_event));
    let turn_sink = Arc::clone(&sink);
    let id = conv.id.clone();
    let res = run_with_registry(
        &mut conv.child,
        question.to_string(),
        Vec::new(),
        &conv.registry,
        move |ev| match ev {
            SessionEvent::LlmUsage {
                total_tokens,
                input_tokens,
                output_tokens,
            } => {
                if let Ok(mut a) = turn_acc.lock() {
                    a.total_tokens += total_tokens;
                    a.rounds += 1;
                }
                // Bare forward: downstream accumulates + persists this exactly
                // like a main-task round. Never wrapped - inside a
                // SidecarChild it would be dropped by the persistence gate
                // and the cost would vanish.
                emit(
                    &turn_sink,
                    SessionEvent::LlmUsage {
                        total_tokens,
                        input_tokens,
                        output_tokens,
                    },
                );
            }
            // Turn boundary belongs to SidecarTurn; a child Done would flip
            // the parent UI into idle mid-sidecar.
            SessionEvent::Done => {}
            SessionEvent::Error(e) => {
                // Not forwarded: a sidecar failure must not terminate or flag
                // the parent run. Surfaced via SidecarTurn { ok: false }.
                if let Ok(mut a) = turn_acc.lock() {
                    if a.error.is_none() {
                        a.error = Some(e);
                    }
                }
            }
            other => emit(
                &turn_sink,
                SessionEvent::SidecarChild {
                    id: id.clone(),
                    ev: Box::new(other),
                },
            ),
        },
    )
    .await;

    let observed = match acc.lock() {
        Ok(g) => g.clone(),
        Err(_) => TurnAcc::default(),
    };
    let ok = res.is_ok() && observed.error.is_none();
    if let Err(e) = &res {
        tracing::warn!(sidecar = %conv.id, error = %e, "sidecar turn failed");
    }
    // Final assistant text from this turn's slice only (see `baseline`).
    let mut answer = conv.child.messages[baseline..]
        .iter()
        .rev()
        .find(|m| m.role == Role::Assistant && !m.text().trim().is_empty())
        .map(|m| m.text())
        .unwrap_or_default();
    if answer.is_empty() {
        if let Some(e) = &observed.error {
            answer = e.clone();
        }
    }
    let elapsed_ms = started.elapsed().as_millis() as u64;
    let turn = SidecarTurn {
        ok,
        answer: answer.clone(),
        elapsed_ms,
        total_tokens: observed.total_tokens,
        rounds: observed.rounds,
    };
    emit(
        &sink,
        SessionEvent::SidecarTurn {
            id: conv.id.clone(),
            ok,
            answer,
            elapsed_ms,
            total_tokens: observed.total_tokens,
            rounds: observed.rounds,
        },
    );
    Ok(turn)
}
