//! B3 regression: the heartbeat budget must be independent of (and far below)
//! the control-plane `READ_TIMEOUT`, and one slow beat must never make a live
//! node converge to `error("node lost")` on the server.
//!
//! - `hung_heartbeat_times_out_within_injected_budget`: a stub that parks
//!   every heartbeat makes `heartbeat()` return `Err` after the INJECTED
//!   budget (100ms) — not after the old 120s `READ_TIMEOUT`.
//! - `heartbeat_recovers_after_timeout`: the SAME uplink serves beats again
//!   once the stub stops wedging; a timed-out beat never poisons the client.
//! - `runner_keeps_beating_after_a_timed_out_beat`: end-to-end — with the
//!   production default budget the runner survives one wedged beat (>=2
//!   heartbeats observed) and still drives its task to terminal `done`.

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_node::uplink::{Uplink, HEARTBEAT_TIMEOUT};
use opencoder_node::NodeOpts;

fn test_opts(base: &str, workdir: &std::path::Path, data: &std::path::Path) -> NodeOpts {
    NodeOpts {
        name: "node-budget".into(),
        remote: base.into(),
        token: support::TOKEN.into(),
        workdir: workdir.to_path_buf(),
        heartbeat_interval: Duration::from_millis(100),
        claim_interval: Duration::from_millis(50),
        version: env!("CARGO_PKG_VERSION").into(),
        local_store_dir: Some(data.to_path_buf()),
        dag: None,
    }
}

/// Pin autopilot off via the project domain file (same reason as the other
/// runner tests: a developer's global `~/.opencoder/ap.json` must not extend
/// the one-round mock).
fn pin_autopilot_off(workdir: &std::path::Path) {
    std::fs::create_dir_all(workdir.join(".opencoder")).unwrap();
    std::fs::write(
        workdir.join(".opencoder").join("ap.json"),
        r#"{"mode":"off"}"#,
    )
    .unwrap();
}

/// A heartbeat endpoint that never answers within the injected budget must
/// surface as a fast `Err` — the caller-side contract is "log a warning and
/// retry next tick", which only works if the call actually returns.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hung_heartbeat_times_out_within_injected_budget() {
    let (base, st) = support::spawn_stub().await;
    st.hang_heartbeats_for(Duration::from_secs(2));
    let uplink =
        Uplink::with_heartbeat_timeout(&base, support::TOKEN, Duration::from_millis(100)).unwrap();

    let t0 = Instant::now();
    let out = uplink.heartbeat(support::STUB_NODE_ID).await;
    let elapsed = t0.elapsed();

    assert!(out.is_err(), "a parked heartbeat must fail, not hang");
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

/// Self-healing: after one timed-out beat the SAME uplink keeps serving. The
/// heartbeat loop owns a long-lived `Uplink`, so a wedged beat must be a
/// per-request event, never a poisoned client.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn heartbeat_recovers_after_timeout() {
    let (base, st) = support::spawn_stub().await;
    st.hang_heartbeats_for(Duration::from_millis(250));
    let uplink =
        Uplink::with_heartbeat_timeout(&base, support::TOKEN, Duration::from_millis(80)).unwrap();

    let t0 = Instant::now();
    assert!(
        uplink.heartbeat(support::STUB_NODE_ID).await.is_err(),
        "wedged beat must time out"
    );
    assert!(
        t0.elapsed() < Duration::from_secs(5),
        "first beat must fail fast, took {:?}",
        t0.elapsed()
    );

    // Once the wedge window lapses the same uplink serves beats normally.
    while Instant::now() < t0 + Duration::from_millis(300) {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let hb = uplink
        .heartbeat(support::STUB_NODE_ID)
        .await
        .expect("recovered beat must succeed");
    assert!(hb.cancel_task_ids.is_empty());
}

/// End-to-end liveness: a beat wedged longer than the production
/// [`HEARTBEAT_TIMEOUT`] costs exactly one logged timeout; the next tick
/// (`MissedTickBehavior::Skip`) lands after the wedge lapses, the stub sees
/// >=2 heartbeats, and the claimed task still finishes `done`. Worst silent
/// gap stays ~timeout + tick interval, inside the server's 20s stale window.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runner_keeps_beating_after_a_timed_out_beat() {
    let (base, st) = support::spawn_stub().await;
    let wedge = HEARTBEAT_TIMEOUT + Duration::from_millis(300);
    assert!(wedge > HEARTBEAT_TIMEOUT, "wedge must outlast the budget");
    st.hang_heartbeats_for(wedge);

    let workdir = tempfile::tempdir().unwrap().keep();
    let data = tempfile::tempdir().unwrap();
    pin_autopilot_off(&workdir);
    let task = st.push_task("say hello twice");

    let client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new().with_default(vec![
        LlmEvent::TextDelta("alpha".into()),
        LlmEvent::Completed {
            text: "alpha".into(),
            tool_calls: vec![],
            usage: None,
        },
    ]));
    let runner = tokio::spawn(opencoder_node::run_node(
        test_opts(&base, &workdir, data.path()),
        Some(client),
    ));

    // The wedged beat (timed out client-side) plus the recovery beat must
    // both reach the stub — the runner did not stall nor give up.
    support::wait_for(30, || {
        let n = st.heartbeat_count();
        (n >= 2).then_some(n)
    })
    .await;

    // ...and the workflow survives: the task is claimed and reported done.
    support::wait_for(60, || st.status_of(&task.task_id)).await;
    assert_eq!(st.status_of(&task.task_id).as_deref(), Some("done"));

    runner.abort();
}
