//! Background subtask watcher: polls the server until every registered
//! one-shot subtask reaches a terminal state, then reports each completion
//! once on the TUI-internal UI channel. The watcher is independent of the
//! session worker (workers are quit on `/task` switches) so writebacks keep
//! flowing into the parent registry that owns the subtask.

use crate::remote::client::ServerClient;
use crate::worker::UiEvent;
use std::time::Duration;
use tokio::sync::mpsc;

/// One subtask to poll. `session` is the parent session whose runtime
/// registry owns the subtask; the emitted update routes back into it even
/// after task switches.
pub(crate) struct WatchTarget {
    pub session: String,
    pub execution_id: String,
}

/// Poll period; matches the fleet heartbeat cadence.
const POLL_INTERVAL: Duration = Duration::from_secs(5);
/// Consecutive failed ROUNDS (any status transport error marks the round)
/// before the watcher gives up and settles every remaining target as errored.
const MAX_FAILED_ROUNDS: u32 = 5;

/// Spawned after every dispatch and on parent-session hydration (the event
/// hub arms it for every pending registry entry).
pub(crate) fn spawn(
    client: ServerClient,
    ui: mpsc::Sender<UiEvent>,
    targets: Vec<WatchTarget>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(run(client, ui, targets))
}

async fn run(client: ServerClient, ui: mpsc::Sender<UiEvent>, mut targets: Vec<WatchTarget>) {
    let mut ticker = tokio::time::interval(POLL_INTERVAL);
    let mut failed_rounds = 0;
    while !targets.is_empty() {
        ticker.tick().await;
        let mut round_error = None;
        let mut remaining = Vec::with_capacity(targets.len());
        for target in std::mem::take(&mut targets) {
            match crate::remote::subtasks::fetch_status(&client, &target.execution_id).await {
                Ok(status) => {
                    if status.terminal() {
                        // Summary/token fetch failures degrade to None/0 —
                        // they never fail the round (status is the truth).
                        let summary =
                            crate::remote::subtasks::fetch_summary(&client, &target.execution_id)
                                .await
                                .ok()
                                .flatten();
                        let tokens =
                            crate::remote::subtasks::fetch_tokens(&client, &target.execution_id)
                                .await
                                .unwrap_or(0);
                        if !send(&ui, &target, status.as_str().to_string(), summary, tokens).await {
                            return; // UI channel closed: nothing left to update
                        }
                    } else {
                        remaining.push(target);
                    }
                }
                Err(error) => {
                    round_error = Some(error);
                    remaining.push(target);
                }
            }
        }
        targets = remaining;
        failed_rounds = if round_error.is_some() {
            failed_rounds + 1
        } else {
            0
        };
        if failed_rounds >= MAX_FAILED_ROUNDS {
            // Transport dead: settle each remaining target once with an
            // error status so its registry entry stops counting as pending.
            let detail = round_error
                .map(|error| format!("{error:#}"))
                .unwrap_or_else(|| "subtask status unreachable".into());
            for target in &targets {
                let done = send(
                    &ui,
                    target,
                    "error".into(),
                    Some(crate::subtask_registry::bound_summary(&detail)),
                    0,
                )
                .await;
                if !done {
                    return;
                }
            }
            return;
        }
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
