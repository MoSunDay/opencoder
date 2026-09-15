//! Recover dispatches whose accepting server died before writing the reply.
use crate::AppState;
use futures::{stream, StreamExt};
use std::{sync::Arc, time::Duration};

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
            let ready: std::collections::HashSet<_> = state
                .hub
                .views()
                .await
                .into_iter()
                .filter(|n| n.online && n.snapshot.as_ref().is_some_and(|s| s.ready))
                .map(|n| n.registration.id)
                .collect();
            let dispatch = stream::iter(assignments.into_iter().filter(|a| ready.contains(&a.index.node_id)))
                .for_each_concurrent(16, |assignment| {
                    let state = state.clone();
                    async move {
                        let id = assignment.index.id.clone();
                        let reply = crate::api::executions::submit(&state,assignment.request).await;
                        if reply.status != 202 { tracing::warn!(%id,status=reply.status,"durable dispatch remains unresolved"); }
                    }
                });
            tokio::select! { _ = dispatch => {}, _ = state.lifecycle.retired() => return }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
}
