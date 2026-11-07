//! Ledger-domain contracts for the rdb (RocksDB) backend, ported from the
//! SQL suites: node registry and identity (`tests/nodes_store*`,
//! `tests/access_tokens`), brain v4 layered scheduler + reporting outbox
//! (`tests/brain_layered_v4*`) and execution inventory + DAG step
//! snapshots (`tests/inventory_sources`, `tests/dag_snapshot`).

#[path = "rdb_ledger/inventory_fixtures.rs"]
mod inventory_fixtures;
#[path = "rdb_ledger/layered_fixtures.rs"]
mod layered_fixtures;
#[path = "rdb_ledger/suite_dag_snapshot.rs"]
mod suite_dag_snapshot;
#[path = "rdb_ledger/suite_identity.rs"]
mod suite_identity;
#[path = "rdb_ledger/suite_inventory.rs"]
mod suite_inventory;
#[path = "rdb_ledger/suite_layered.rs"]
mod suite_layered;
#[path = "rdb_ledger/suite_nodes.rs"]
mod suite_nodes;
#[path = "rdb_ledger/suite_reporting.rs"]
mod suite_reporting;

use opencoder_core::identity::{token_hash, Role};
use opencoder_store::rdb_store::RdbStore;
use opencoder_store::Store;

pub(crate) async fn mem() -> RdbStore {
    RdbStore::open_memory().await.unwrap()
}

pub(crate) async fn register(
    store: &RdbStore,
    name: &str,
    now_ms: i64,
) -> opencoder_store::NodeRecord {
    store
        .register_node(name, Some("v1"), Some("/tmp/wd"), None, now_ms)
        .await
        .unwrap()
}
