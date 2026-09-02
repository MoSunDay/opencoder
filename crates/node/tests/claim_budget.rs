//! Regression for the idle-claim liveness hole: `claim_next` used to run
//! with no budget of its own, so on the idle runner's `select!` (heartbeat
//! and claim arms share one loop) a server that accepted the claim request
//! but never answered kept the node silent for up to the 120s control-plane
//! `READ_TIMEOUT` — far past the server liveness window (`STALE_AFTER_MS =
//! 20s`) — making a live idle node look lost.
//!
//! - `hung_claim_times_out_within_injected_budget`: a stub that parks every
//!   claim poll makes `claim_next()` return `Err` after the INJECTED budget
//!   (100ms) — not after the old 120s `READ_TIMEOUT`.
//! - `claim_recovers_after_timeout`: the SAME uplink serves polls again once
//!   the stub stops wedging; a timed-out poll never poisons the client.

mod support;

use std::time::{Duration, Instant};

use opencoder_node::uplink::Uplink;

/// A claim endpoint that never answers within the injected budget must
/// surface as a fast `Err` — the runner's caller-side contract is "log a
/// warning and retry next tick", which only works if the call actually
/// returns while the heartbeat arm is blocked behind it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hung_claim_times_out_within_injected_budget() {
    let (base, st) = support::spawn_stub().await;
    st.hang_claims_for(Duration::from_secs(2));
    let uplink = Uplink::with_timeouts(
        &base,
        support::TOKEN,
        Duration::from_secs(5),
        Duration::from_millis(100),
    )
    .unwrap();

    let t0 = Instant::now();
    let out = uplink.claim_next(support::STUB_NODE_ID).await;
    let elapsed = t0.elapsed();

    assert!(out.is_err(), "a parked claim must fail, not hang");
    assert!(
        elapsed < Duration::from_secs(5),
        "timeout must follow the injected budget, got {elapsed:?} (old control-plane budget: 120s)"
    );
    assert!(
        elapsed >= Duration::from_millis(90),
        "timeout fired early: {elapsed:?} vs injected 100ms budget"
    );
    let err = out.unwrap_err().to_string();
    assert!(
        err.contains("timed out"),
        "error must name the timeout, got: {err}"
    );
}

/// Self-healing: after one timed-out poll the SAME uplink keeps serving —
/// including a real claim once a task is queued. The idle loop owns a
/// long-lived `Uplink`, so a wedged poll must be a per-request event, never
/// a poisoned client.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn claim_recovers_after_timeout() {
    let (base, st) = support::spawn_stub().await;
    st.hang_claims_for(Duration::from_millis(250));
    let uplink = Uplink::with_timeouts(
        &base,
        support::TOKEN,
        Duration::from_secs(5),
        Duration::from_millis(80),
    )
    .unwrap();

    let t0 = Instant::now();
    assert!(
        uplink.claim_next(support::STUB_NODE_ID).await.is_err(),
        "wedged poll must time out"
    );
    assert!(
        t0.elapsed() < Duration::from_secs(5),
        "first poll must fail fast, took {:?}",
        t0.elapsed()
    );

    // Once the wedge window lapses the same uplink serves polls normally —
    // and hands out an enqueued task through the 204/JSON envelope path.
    while Instant::now() < t0 + Duration::from_millis(300) {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let queued = st.push_task("after the wedge");
    let claimed = uplink.claim_next(support::STUB_NODE_ID).await;
    let claimed = claimed.expect("same uplink must serve polls after a timeout");
    assert_eq!(
        claimed.task.expect("queued task must be claimed").task_id,
        queued.task_id,
        "recovery must preserve the FIFO claim contract"
    );
}
