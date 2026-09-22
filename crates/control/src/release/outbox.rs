//! Recover dispatches whose accepting server died before writing the reply.
use crate::AppState;
use futures::{stream, StreamExt};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

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
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            };
            after = assignments
                .last()
                .map(|a| a.index.id.clone())
                .unwrap_or_default();
            if assignments.is_empty() {
                retries.finish_scan();
            } else {
                retries.observe(assignments.iter().map(|a| a.index.id.clone()));
            }
            let ready: std::collections::HashMap<_, _> = state
                .hub
                .views()
                .await
                .into_iter()
                .filter_map(|node| {
                    let snapshot = node.snapshot?;
                    (node.online && snapshot.ready)
                        .then_some((node.registration.id, snapshot.generation))
                })
                .collect();
            let now = Instant::now();
            let due: Vec<_> = assignments
                .into_iter()
                .filter_map(|assignment| {
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
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
}
