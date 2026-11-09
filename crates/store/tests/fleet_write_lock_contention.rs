//! Cross-process write-lock contention exercises the bounded Busy retry on a
//! fleet write path: an external database holds the write lease past the
//! pooled connection's entire busy budget, so the write can only commit by
//! retrying the contended BEGIN.
use opencoder_core::fleet::*;
use opencoder_store::fleet::FleetStore;
use std::time::Duration;

/// Pooled connections keep a 30 s busy budget; turso takes the cross-process
/// write lock with one try per busy cycle, so an unprotected transaction
/// fails with `Busy` once the lease outlives that budget.
const LEASE_MS: u64 = 31_000;

/// Hold the fleet database's cross-process write lease for [`LEASE_MS`] from
/// an independent database handle. Two turso databases on one file reproduce
/// the cross-process conflict (the OFD locks are per open file description).
async fn external_write_lease(path: &str) -> tokio::task::JoinHandle<()> {
    let holder = turso::Builder::new_local(path)
        .experimental_multiprocess_wal(true)
        .build()
        .await
        .unwrap()
        .connect()
        .unwrap();
    holder.busy_timeout(Duration::from_secs(35)).unwrap();
    holder.execute("BEGIN IMMEDIATE", ()).await.unwrap();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(LEASE_MS)).await;
        holder.execute("COMMIT", ()).await.unwrap();
    })
}

/// The report below began with a single non-retrying `BEGIN IMMEDIATE` before
/// the bounded retry: under this lease that begin fails with `Busy` at the
/// busy budget. Through the retry the transaction queues past the lease,
/// commits, and the reported row is durable.
#[tokio::test]
async fn fleet_report_queues_past_an_external_write_lease() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("contended-fleet.db");
    let store = FleetStore::open(&path).await.unwrap();
    let path = path.to_str().unwrap().to_owned();

    let record = ExecutionIndex {
        id: "agent-contended".into(),
        created_at: 1,
        kind: ExecutionKind::Agent,
        node_id: "node-holder".into(),
        status: ExecutionStatus::Running,
    };
    let lease = external_write_lease(&path).await;
    store
        .apply_index_report("node-holder", &[record], None)
        .await
        .expect("the report transaction must retry past the external lease");
    lease.await.unwrap();

    let landed = store
        .index("agent-contended")
        .await
        .unwrap()
        .expect("reported row landed");
    assert_eq!(landed.status, ExecutionStatus::Running);
    assert_eq!(landed.node_id, "node-holder");
    assert_eq!(landed.kind, ExecutionKind::Agent);
}
