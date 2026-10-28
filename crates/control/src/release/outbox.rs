//! Recover dispatches whose accepting server died before writing the reply.
use crate::AppState;
use futures::{stream, StreamExt};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

mod resources;
mod retry;

pub fn start(state: &Arc<AppState>) {
    if state
        .lifecycle
        .outbox_started
        .swap(true, std::sync::atomic::Ordering::SeqCst)
    {
        return;
    }
    let weak = Arc::downgrade(state);
    tokio::spawn(async move {
        let mut after = String::new();
        let mut retries = retry::Retries::default();
        let mut resources = Vec::new();
        let mut consecutive_idle: u32 = 0;
        loop {
            let Some(state) = weak.upgrade() else {
                return;
            };
            if state
                .lifecycle
                .retiring
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                return;
            }
            let assignments = match state.fleet.pending_assignments(&after, 128).await {
                Ok(assignments) => assignments,
                Err(error) => {
                    tracing::error!(%error, "read durable dispatch outbox");
                    // An errored poll is not an idle observation; restart the
                    // ladder so the retry stays conservative.
                    consecutive_idle = 0;
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            };
            let had_assignments = !assignments.is_empty();
            after = assignments
                .last()
                .map(|a| a.index.id.clone())
                .unwrap_or_default();
            if assignments.is_empty() {
                retries.finish_scan();
            } else {
                retries.observe(assignments.iter().map(|a| a.index.id.clone()));
            }
            // The hub view scan is only built when this tick can act on it:
            // a pending page to recover, or queued resource grants to admit
            // once the durable scan completes. Purely idle ticks skip it.
            let ready: std::collections::HashMap<_, _> =
                if assignments.is_empty() && resources.is_empty() {
                    Default::default()
                } else {
                    state
                        .hub
                        .views()
                        .await
                        .into_iter()
                        .filter_map(|node| {
                            let snapshot = node.snapshot?;
                            (node.online && snapshot.ready)
                                .then_some((node.registration.id, snapshot.generation))
                        })
                        .collect()
                };
            let now = Instant::now();
            let due: Vec<_> = assignments
                .into_iter()
                .filter_map(|assignment| {
                    if assignment.request.input.get("_resource_request").is_some() {
                        resources.push((assignment.index.created_at, assignment.index.id));
                        return None;
                    }
                    let generation = ready.get(&assignment.index.node_id)?;
                    retries
                        .ready(&assignment.index.id, generation, now)
                        .then_some((assignment, generation.clone()))
                })
                .collect();
            let dispatch = stream::iter(due).map(|(assignment, generation)| {
                let state = state.clone();
                async move {
                    let id = assignment.index.id.clone();
                    // Recovery must preserve the frozen private grant as well
                    // as the public request and its idempotency fingerprint.
                    let reply = crate::api::executions::submit_private(
                        &state, assignment.request, assignment.private_context,
                    ).await;
                    if reply.status != 202 {
                        tracing::warn!(%id,status=reply.status,"durable dispatch remains unresolved");
                    }
                    (id, generation, reply.status, Instant::now())
                }
            }).buffer_unordered(16).collect::<Vec<_>>();
            let completed = tokio::select! {
                completed = dispatch => completed,
                _ = state.lifecycle.retired() => return,
            };
            for (id, generation, status, completed_at) in completed {
                retries.completed(id, generation, status, completed_at);
            }
            // Resource admission is serialized in durable submission order over
            // the complete scan. New API submissions cannot bypass this queue.
            if after.is_empty() {
                tokio::select! {
                    _ = resources::dispatch(&state, std::mem::take(&mut resources), &ready, &mut retries) => {},
                    _ = state.lifecycle.retired() => return,
                }
            }
            // Retry backoffs are >=1s (see retry::Retries::completed), so the
            // idle ladder never delays recovery; active draining keeps a 100ms
            // cadence so new submissions are dispatched promptly.
            let idle =
                !had_assignments && after.is_empty() && retries.idle() && resources.is_empty();
            let delay = if idle {
                let delay = idle_delay(consecutive_idle);
                consecutive_idle = consecutive_idle.saturating_add(1);
                delay
            } else {
                consecutive_idle = 0;
                Duration::from_millis(100)
            };
            tokio::time::sleep(delay).await;
        }
    });
}

/// Idle poll backoff ladder: consecutive idle polls grow the sleep 1s → 2s
/// → 5s (cap); any observed work resets the ladder via the 100ms drain
/// cadence. Worst-case recovery delay +5s, far under the 30s dispatch budget.
fn idle_delay(consecutive_idle: u32) -> Duration {
    match consecutive_idle {
        0 => Duration::from_secs(1),
        1 => Duration::from_secs(2),
        _ => Duration::from_secs(5),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_ladder_grows_one_two_five_and_caps_at_five() {
        assert_eq!(idle_delay(0), Duration::from_secs(1));
        assert_eq!(idle_delay(1), Duration::from_secs(2));
        assert_eq!(idle_delay(2), Duration::from_secs(5));
        assert_eq!(idle_delay(10), Duration::from_secs(5));
    }
}
