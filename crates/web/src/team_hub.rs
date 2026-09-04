//! In-process registry of RUNNING team-topic runtimes, keyed by topic id.
//!
//! A topic runtime is a spawned tokio task driving
//! [`opencoder_team::run_topic`]; the only cross-task handle we need is its
//! cooperative [`CancelToken`] (the runtime itself persists all state on the
//! team share). The hub is a tiny map: `register` before spawn,
//! `hub.remove(topic_id)` when the task ends — an entry therefore means
//! "a runtime task is alive right now", never "the topic is unfinished" (a
//! server restart leaves executing topics with no entry; those are resume
//! candidates and cancel falls back to the disk path in `api_teams.rs`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use opencoder_team::CancelToken;

use crate::AppState;

/// One entry per live topic-runtime task: just the cancel handle.
#[derive(Default)]
pub struct TeamHub {
    inner: Mutex<HashMap<String, CancelToken>>,
}

impl TeamHub {
    pub fn new() -> Self {
        TeamHub::default()
    }

    /// Register a fresh token for `topic_id` and return it. A leftover entry
    /// (a previous runtime whose task has not reaped itself yet — e.g. a
    /// resume racing the old task's exit) is cancelled FIRST and replaced so
    /// the map can never leak stale tokens.
    pub fn register(&self, topic_id: &str) -> CancelToken {
        let token = CancelToken::new();
        let mut map = self.inner.lock().expect("team hub lock");
        if let Some(stale) = map.insert(topic_id.to_string(), token.clone()) {
            stale.cancel();
        }
        token
    }

    /// Cancel the runtime of `topic_id` if one is registered. `true` means
    /// the token existed and is now cancelled (the runtime persists the
    /// terminal state itself); `false` means no in-process runtime owns it.
    pub fn cancel(&self, topic_id: &str) -> bool {
        let map = self.inner.lock().expect("team hub lock");
        match map.get(topic_id) {
            Some(token) => {
                token.cancel();
                true
            }
            None => false,
        }
    }

    /// Whether an in-process runtime task is currently registered.
    pub fn is_running(&self, topic_id: &str) -> bool {
        self.inner
            .lock()
            .expect("team hub lock")
            .contains_key(topic_id)
    }

    /// Drop the entry (called by the runtime task itself on exit, whatever
    /// the outcome — terminal state is already on disk by then).
    pub fn remove(&self, topic_id: &str) {
        self.inner.lock().expect("team hub lock").remove(topic_id);
    }
}

/// Spawn the topic runtime for (`team_name`, `topic_id`) on this server:
/// register a cancel token, run [`opencoder_team::run_topic`] to a terminal
/// state (idempotent, doubles as resume), then unregister.
///
/// Errors are ONLY logged here: `run_topic` persists `finished(error)`
/// itself, so the HTTP layer must never block on the outcome. The response
/// that triggered the spawn has already been answered.
pub fn spawn_topic_runtime(state: Arc<AppState>, team_name: String, topic_id: String) {
    let store = state.store.clone();
    let dispatcher = state.team.dispatcher.clone();
    let cfg = state.team.run.clone();
    let team_state = state.team.clone();
    let token = team_state.hub.register(&topic_id);
    tokio::spawn(async move {
        let outcome =
            opencoder_team::run_topic(store, dispatcher, &cfg, &team_name, &topic_id, token).await;
        if let Err(error) = outcome {
            tracing::error!(
                team = %team_name,
                topic = %topic_id,
                error = %format!("{error:#}"),
                "team topic runtime failed (terminal state already persisted by run_topic)"
            );
        }
        team_state.hub.remove(&topic_id);
    });
}

#[cfg(test)]
mod tests {
    use super::TeamHub;

    #[test]
    fn register_then_is_running_cancel_and_remove() {
        let hub = TeamHub::new();
        assert!(!hub.is_running("t1"), "empty hub runs nothing");

        let token = hub.register("t1");
        assert!(!token.is_cancelled());
        assert!(hub.is_running("t1"), "register marks the topic running");

        assert!(hub.cancel("t1"), "cancel hits the live entry");
        assert!(token.is_cancelled(), "the runtime's token is flipped");
        // Entry stays until the task removes it (cancel ≠ unregister).
        assert!(hub.is_running("t1"));

        hub.remove("t1");
        assert!(!hub.is_running("t1"), "remove unregisters the runtime");
        assert!(!hub.cancel("t1"), "cancel after remove misses");
    }

    #[test]
    fn re_register_cancels_the_stale_token_and_replaces_it() {
        let hub = TeamHub::new();
        let first = hub.register("t1");
        let second = hub.register("t1");
        assert!(first.is_cancelled(), "stale token must die on re-register");
        assert!(!second.is_cancelled(), "the new runtime starts fresh");
        assert!(hub.cancel("t1"));
        assert!(second.is_cancelled());
        assert!(first.is_cancelled(), "the stale token stays cancelled");
    }

    #[test]
    fn unknown_topic_operations_are_noops() {
        let hub = TeamHub::new();
        assert!(!hub.cancel("ghost"));
        assert!(!hub.is_running("ghost"));
        hub.remove("ghost"); // must not panic
    }
}
