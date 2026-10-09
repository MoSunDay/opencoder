//! External grants are attempted by the Server queue, oldest first.
use super::retry::Retries;
use crate::AppState;
use std::{collections::HashMap, sync::Arc, time::Instant};

pub(super) async fn dispatch(
    state: &Arc<AppState>,
    mut pending: Vec<(i64, String)>,
    ready: &HashMap<String, String>,
    retries: &mut Retries,
) {
    pending.sort();
    pending.dedup();
    for (_, id) in pending {
        let assignment = match state.fleet.assignment(&id).await {
            Ok(Some(value)) => value,
            _ => continue,
        };
        let Some(generation) = ready.get(&assignment.index.node_id) else {
            continue;
        };
        if !retries.ready(&id, generation, Instant::now()) {
            continue;
        }
        let reply = crate::api::executions::dispatch_queued(
            state,
            assignment.request,
            assignment.private_context,
        )
        .await;
        if reply.status == 202 {
            retries.deferred(id, generation.clone(), Instant::now());
        } else {
            retries.completed(id, generation.clone(), reply.status, Instant::now());
        }
    }
}
