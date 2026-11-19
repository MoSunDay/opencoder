//! Background subtask watcher: polls the server until every registered
//! one-shot subtask finishes — a terminal status or the idle convergence of
//! conversational kinds (mapped via `subtask_registry::settle_status`) — then
//! reports each completion once on the TUI-internal UI channel. The watcher
//! is independent of the session worker (workers are quit on `/task`
//! switches) so writebacks keep flowing into the parent registry that owns
//! the subtask.
//!
//! Failures are isolated per target: a transport error only advances that
//! target's consecutive-failure counter, so one unreachable execution never
//! drags healthy siblings down with it.

use crate::remote::client::ServerClient;
use crate::worker::UiEvent;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

/// Ids settled locally (terminal cancel response): the watcher drops them
/// before polling and before delivering a terminal update, so a dead target
/// can never overwrite the settled status with `error`.
pub(crate) type SettledSet = Arc<Mutex<HashSet<String>>>;

/// `true` when `id` was settled locally; poisoned locks are recovered.
fn is_settled(settled: &SettledSet, id: &str) -> bool {
    settled
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(id)
}

/// One subtask to poll. `session` is the parent session whose runtime
/// registry owns the subtask; the emitted update routes back into it even
/// after task switches.
pub(crate) struct WatchTarget {
    pub session: String,
    pub execution_id: String,
}

/// Per-target poll bookkeeping: consecutive failures and the last error
/// detail (used verbatim for the settling "error" delivery).
struct TargetState {
    target: WatchTarget,
    failed_rounds: u32,
    last_error: Option<String>,
}

/// Poll period; matches the fleet heartbeat cadence.
const POLL_INTERVAL: Duration = Duration::from_secs(5);
/// Consecutive failed polls per target; only that target settles as error,
/// healthy targets keep polling.
const MAX_FAILED_ROUNDS: u32 = 5;

/// Spawned after every dispatch and on parent-session hydration (the event
/// hub arms it for every pending registry entry). Test/back-compat wrapper
/// without a locally-settled set.
#[allow(dead_code)]
pub(crate) fn spawn(
    client: ServerClient,
    ui: mpsc::Sender<UiEvent>,
    targets: Vec<WatchTarget>,
) -> tokio::task::JoinHandle<()> {
    spawn_with_settled(client, ui, targets, SettledSet::default())
}

/// Production entry: shares the hub's locally-settled id set.
pub(crate) fn spawn_with_settled(
    client: ServerClient,
    ui: mpsc::Sender<UiEvent>,
    targets: Vec<WatchTarget>,
    settled: SettledSet,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(run(client, ui, targets, settled))
}

async fn run(
    client: ServerClient,
    ui: mpsc::Sender<UiEvent>,
    targets: Vec<WatchTarget>,
    settled: SettledSet,
) {
    run_with_settled(client, ui, targets, POLL_INTERVAL, settled).await
}

/// Poll loop with an injectable cadence (tests use a short real-time
/// interval); no locally-settled ids.
#[allow(dead_code)]
pub(crate) async fn run_with(
    client: ServerClient,
    ui: mpsc::Sender<UiEvent>,
    targets: Vec<WatchTarget>,
    interval: Duration,
) {
    run_with_settled(client, ui, targets, interval, SettledSet::default()).await
}

/// Poll loop with an injectable cadence (tests use a short real-time
/// interval). Each round polls every still-live target independently and
/// drops ids settled locally by the cancel-response path.
pub(crate) async fn run_with_settled(
    client: ServerClient,
    ui: mpsc::Sender<UiEvent>,
    targets: Vec<WatchTarget>,
    interval: Duration,
    settled: SettledSet,
) {
    let mut ticker = tokio::time::interval(interval);
    let mut states: Vec<TargetState> = targets
        .into_iter()
        .map(|target| TargetState {
            target,
            failed_rounds: 0,
            last_error: None,
        })
        .collect();
    while !states.is_empty() {
        // Locally settled ids (terminal cancel response) stop being polled.
        states.retain(|state| !is_settled(&settled, &state.target.execution_id));
        if states.is_empty() {
            return;
        }
        ticker.tick().await;
        let mut remaining = Vec::with_capacity(states.len());
        for mut state in std::mem::take(&mut states) {
            let id = state.target.execution_id.clone();
            match crate::remote::subtasks::fetch_status(&client, &id).await {
                Ok(status) => {
                    if let Some(final_status) = crate::subtask_registry::settle_status(status) {
                        // Summary/token fetch failures degrade to None/0 —
                        // they never fail the poll (status is the truth).
                        let summary = crate::remote::subtasks::fetch_summary(&client, &id)
                            .await
                            .ok()
                            .flatten();
                        let tokens = crate::remote::subtasks::fetch_tokens(&client, &id)
                            .await
                            .unwrap_or(0);
                        if is_settled(&settled, &id) {
                            continue; // settled locally mid-round: drop it
                        }
                        if !send(
                            &ui,
                            &state.target,
                            final_status.to_string(),
                            summary,
                            tokens,
                        )
                        .await
                        {
                            return; // UI channel closed: nothing left to update
                        }
                    } else {
                        state.failed_rounds = 0;
                        state.last_error = None;
                        remaining.push(state);
                    }
                }
                Err(error) => {
                    state.failed_rounds += 1;
                    state.last_error = Some(format!("{error:#}"));
                    if state.failed_rounds >= MAX_FAILED_ROUNDS {
                        // This target's transport is dead: settle it once so
                        // its registry entry stops counting as pending. The
                        // other targets are unaffected and keep polling.
                        let detail = state
                            .last_error
                            .as_deref()
                            .unwrap_or("subtask status unreachable");
                        let summary = Some(crate::subtask_registry::bound_summary(detail));
                        if is_settled(&settled, &id) {
                            continue; // settled locally mid-round: no error delivery
                        }
                        if !send(&ui, &state.target, "error".into(), summary, 0).await {
                            return;
                        }
                    } else {
                        remaining.push(state);
                    }
                }
            }
        }
        states = remaining;
    }
}

/// Deliver one update; `false` means the UI channel is closed.
async fn send(
    ui: &mpsc::Sender<UiEvent>,
    target: &WatchTarget,
    status: String,
    summary: Option<String>,
    tokens: u64,
) -> bool {
    ui.send(UiEvent::SubtaskUpdate {
        session: target.session.clone(),
        id: target.execution_id.clone(),
        status,
        summary,
        tokens,
    })
    .await
    .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(id: &str) -> WatchTarget {
        WatchTarget {
            session: "parent-1".into(),
            execution_id: id.into(),
        }
    }

    /// A dead server address makes every status poll a transport error; after
    /// five failed rounds the watcher must settle BOTH remaining targets with
    /// an "error" status and exit (paused clock fast-forwards the 5s polls).
    #[tokio::test(start_paused = true)]
    async fn settles_all_targets_as_error_after_five_failed_rounds() {
        let client = ServerClient::new("http://127.0.0.1:1", None).unwrap();
        let (tx, mut rx) = mpsc::channel(64);
        let handle = spawn(client, tx, vec![target("agent-a"), target("agent-b")]);
        let mut settled = Vec::new();
        while let Some(event) = rx.recv().await {
            let UiEvent::SubtaskUpdate {
                session,
                id,
                status,
                summary,
                tokens,
            } = event
            else {
                continue;
            };
            assert_eq!(session, "parent-1");
            assert_eq!(status, "error");
            assert_eq!(tokens, 0);
            assert!(summary.is_some_and(|text| !text.is_empty()));
            settled.push(id);
            if settled.len() == 2 {
                break;
            }
        }
        assert_eq!(settled.len(), 2, "one update per remaining target");
        assert!(settled.contains(&"agent-a".to_string()));
        assert!(settled.contains(&"agent-b".to_string()));
        assert!(
            tokio::time::timeout(Duration::from_secs(1), handle)
                .await
                .is_ok(),
            "watcher exits after settling"
        );
    }

    /// A target settled locally (terminal cancel response) is dropped before
    /// the first poll: no `/index` call, no delivery, and the watcher exits.
    #[tokio::test]
    async fn watcher_skips_locally_settled_target() {
        let fixture = crate::remote::tests::support::Fixture::new().await;
        let client = ServerClient::new(&fixture.url, None).unwrap();
        let (tx, mut rx) = mpsc::channel(16);
        let settled: SettledSet = Arc::new(Mutex::new(HashSet::from(["agent-sub-1".to_string()])));
        let handle = spawn_with_settled(client, tx, vec![target("agent-sub-1")], settled);
        assert!(
            tokio::time::timeout(Duration::from_secs(1), handle)
                .await
                .is_ok(),
            "settled target is dropped without polling"
        );
        assert!(rx.try_recv().is_err(), "no update for a settled target");
        assert_eq!(
            fixture
                .state
                .lock()
                .unwrap()
                .index_calls
                .get("agent-sub-1")
                .copied()
                .unwrap_or(0),
            0,
            "a settled target is never polled"
        );
    }

    /// The watcher exits immediately when spawned without targets and emits
    /// nothing.
    #[tokio::test(start_paused = true)]
    async fn exits_immediately_without_targets() {
        let client = ServerClient::new("http://127.0.0.1:1", None).unwrap();
        let (tx, mut rx) = mpsc::channel(64);
        let handle = spawn(client, tx, Vec::new());
        assert!(
            tokio::time::timeout(Duration::from_secs(1), handle)
                .await
                .is_ok(),
            "empty target list exits immediately"
        );
        assert!(rx.try_recv().is_err(), "no updates without targets");
    }
}
