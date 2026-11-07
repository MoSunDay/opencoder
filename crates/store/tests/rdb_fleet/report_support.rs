//! Shared fixtures for the node index report suites.
use opencoder_core::fleet::{ExecutionIndex, ExecutionKind, ExecutionStatus};
use opencoder_store::fleet::FleetStore;

pub(crate) fn agent(id: &str, created_at: i64, status: ExecutionStatus) -> ExecutionIndex {
    ExecutionIndex {
        id: id.into(),
        created_at,
        kind: ExecutionKind::Agent,
        node_id: "node-a".into(),
        status,
    }
}

pub(crate) async fn stored(store: &FleetStore, id: &str) -> ExecutionIndex {
    store.index(id).await.unwrap().unwrap()
}
