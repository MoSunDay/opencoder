//! Mid-turn model selection queue: a frontend hands over `(mode, model)`
//! picks while a turn runs and the runner applies them at the next completed
//! LLM round. The handle is plain shared state, so a frontend never needs the
//! serial worker command channel (which is only drained between turns).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use opencoder_core::ConfigMode;

use crate::{runner::SessionEvent, SessionState};

/// Shared interior-mutable FIFO of session-only `(mode, model)` picks. Cloned
/// to the frontend before [`SessionState`] moves into the runner; pushes are
/// non-blocking and the runner is the only drainer.
#[derive(Clone, Default)]
pub struct ModelSwitchQueue(Arc<Mutex<VecDeque<(ConfigMode, String)>>>);

/// Append one pick. The runner applies it after the current LLM round.
pub fn queue_model_switch(queue: &ModelSwitchQueue, mode: ConfigMode, model: impl Into<String>) {
    if let Ok(mut pending) = queue.0.lock() {
        pending.push_back((mode, model.into()));
    }
}

/// Take every pending pick in submission order.
pub fn drain_model_switches(queue: &ModelSwitchQueue) -> Vec<(ConfigMode, String)> {
    match queue.0.lock() {
        Ok(mut pending) => pending.drain(..).collect(),
        Err(_) => Vec::new(),
    }
}

/// Number of picks not applied yet.
pub fn pending_model_switches(queue: &ModelSwitchQueue) -> usize {
    queue.0.lock().map(|pending| pending.len()).unwrap_or(0)
}

/// Wire key of a mode (`"shared"` / `"act"` / `"plan"`).
pub fn mode_key(mode: ConfigMode) -> &'static str {
    match mode {
        ConfigMode::Shared => "shared",
        ConfigMode::Act => "act",
        ConfigMode::Plan => "plan",
    }
}

/// Inverse of [`mode_key`]; `None` for any other string.
pub fn mode_from_key(key: &str) -> Option<ConfigMode> {
    match key {
        "shared" => Some(ConfigMode::Shared),
        "act" => Some(ConfigMode::Act),
        "plan" => Some(ConfigMode::Plan),
        _ => None,
    }
}

/// Apply every queued pick once an LLM round completed. Each pick goes through
/// the same persisted path as an idle `/model` pick. The effective model of
/// the current agent is announced first (`ModelSwitch`, only when it changed),
/// then the pick itself (`ModelOverrideApplied`) so a display surface commits
/// the exact mode it landed in. A failed pick changes nothing, carries its
/// error, and does not stop the picks queued behind it.
pub(crate) async fn apply_queued_model_switches(
    session: &mut SessionState,
    on_event: &mut (dyn FnMut(SessionEvent) + Send),
) {
    for (mode, model) in drain_model_switches(&session.model_switch_queue) {
        let result = session.set_mode_model_override(mode, model.clone()).await;
        if let Some(event) = session.take_model_switch() {
            on_event(event);
        }
        let error = result.err().map(|error| format!("{error:#}"));
        on_event(SessionEvent::ModelOverrideApplied {
            mode: mode_key(mode).to_string(),
            model,
            error,
        });
    }
}

#[cfg(test)]
#[path = "model_switch_tests.rs"]
mod tests;
