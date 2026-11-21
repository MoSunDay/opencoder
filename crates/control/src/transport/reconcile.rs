//! Periodic reservation reconciliation. Bootstrap re-reserves every persisted
//! Pending execution so concurrent admissions do not double-book node loops,
//! but a reservation whose execution has since left Pending (dispatched,
//! terminal or deleted) has no other release path when the acknowledge or RPC
//! failure routes miss it. This loop mirrors the persisted Pending set onto
//! the hub once a minute and hands orphan reservations back, keeping
//! `reserved_loops` honest for node selection without touching the store.
use crate::AppState;
use opencoder_core::fleet::{ExecutionIndex, ExecutionStatus};
use std::{
    collections::HashSet,
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

/// Reservation reconciliation cadence.
const RECONCILE_SECS: u64 = 60;

/// Same cap bootstrap uses for the Pending re-reserve pass; a full page
/// means the Pending set is incomplete, so reconciling could drop live
/// reservations — skip the round instead.
const PENDING_PAGE_CAP: usize = 10000;

pub fn start(state: &Arc<AppState>) {
    let weak = Arc::downgrade(state);
    tokio::spawn(async move {
        loop {
            let Some(state) = weak.upgrade() else {
                return;
            };
            if state.lifecycle.retiring.load(Ordering::SeqCst) {
                return;
            }
            if let Err(error) = reconcile_once(&state).await {
                tracing::error!(%error, "reservation reconcile round failed");
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(RECONCILE_SECS)) => {}
                _ = state.lifecycle.retired() => return,
            }
        }
    });
}

/// One reconciliation pass. A store read failure only delays this round; the
/// reservations stay in place and the next pass retries.
async fn reconcile_once(state: &Arc<AppState>) -> anyhow::Result<()> {
    let indexes = state
        .fleet
        .indexes(None, None, PENDING_PAGE_CAP as u32)
        .await?;
    if indexes.len() >= PENDING_PAGE_CAP {
        tracing::warn!(
            count = indexes.len(),
            "pending page full; skipping reservation reconcile"
        );
        return Ok(());
    }
    let pending = pending_ids(&indexes);
    let released = state
        .hub
        .reconcile_reservations(&pending, opencoder_core::message::now_ms())
        .await;
    if released > 0 {
        tracing::info!(released, "released orphan node reservations");
    }
    Ok(())
}

/// Pure projection of a store page onto its Pending id set.
fn pending_ids(indexes: &[ExecutionIndex]) -> HashSet<String> {
    indexes
        .iter()
        .filter(|index| index.status == ExecutionStatus::Pending)
        .map(|index| index.id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencoder_core::fleet::ExecutionKind;

    fn index(id: &str, status: ExecutionStatus) -> ExecutionIndex {
        ExecutionIndex {
            id: id.into(),
            created_at: 1,
            kind: ExecutionKind::Agent,
            node_id: "node-a".into(),
            status,
        }
    }

    #[test]
    fn pending_ids_keeps_only_pending_entries() {
        let indexes = vec![
            index("agent-pending", ExecutionStatus::Pending),
            index("agent-running", ExecutionStatus::Running),
            index("agent-done", ExecutionStatus::Done),
        ];
        let ids = pending_ids(&indexes);
        assert_eq!(ids.len(), 1);
        assert!(ids.contains("agent-pending"));
    }

    #[test]
    fn pending_ids_of_an_empty_page_is_empty() {
        assert!(pending_ids(&[]).is_empty());
    }
}
