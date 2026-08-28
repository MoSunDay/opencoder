//! Cancellation path: while a task's LLM call hangs open (deterministic
//! pause), the stub starts returning `cancel_task_ids=[tid]` on heartbeat —
//! the executor must converge through the session's own interrupt machinery
//! and report `cancelled`.
//!
//! Deterministic strategy: `MockChatClient::push_hang` parks the ONE queued
//! chat_stream call on an explicit [`tokio::sync::Notify`] permit. The cancel
//! order is only armed AFTER the claim is observed, so no heartbeat can
//! collapse the pending task early; release of the hang happens at teardown.

mod support;

use std::sync::Arc;
use std::time::Duration;

use opencoder_llm::{ChatStream, MockChatClient};
use opencoder_node::NodeOpts;

fn test_opts(base: &str, workdir: &std::path::Path, data: &std::path::Path) -> NodeOpts {
    NodeOpts {
        name: "node-cancel".into(),
        remote: base.into(),
        token: support::TOKEN.into(),
        workdir: workdir.to_path_buf(),
        // Heartbeat FASTER than the run startup sequence needs to first
        // observe a claim: once the task is claimed and running, the very
        // next tick (<200ms) sees the cancel instruction.
        heartbeat_interval: Duration::from_millis(60),
        claim_interval: Duration::from_millis(25),
        version: env!("CARGO_PKG_VERSION").into(),
        local_store_dir: Some(data.to_path_buf()),
    }
}

fn pin_autopilot_off(workdir: &std::path::Path) {
    std::fs::create_dir_all(workdir.join(".opencoder")).unwrap();
    std::fs::write(
        workdir.join(".opencoder").join("ap.json"),
        r#"{"mode":"off"}"#,
    )
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn heartbeat_cancellation_reports_cancelled() {
    let (base, st) = support::spawn_stub().await;
    let workdir = tempfile::tempdir().unwrap().keep();
    let data = tempfile::tempdir().unwrap();
    pin_autopilot_off(&workdir);

    let task = st.push_task("hang forever until cancelled");
    // Hang BEFORE cancellation exists: claim is the arming barrier.
    let notify = Arc::new(tokio::sync::Notify::new());
    let client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new().push_hang(notify.clone()));

    let runner = tokio::spawn(opencoder_node::run_node(
        test_opts(&base, &workdir, data.path()),
        Some(client),
    ));

    // Barrier 1: wait for the CLAIM (task transitioned running server-side).
    // Budgets are generous failure-detection ceilings only: the happy path
    // settles in well under a second, but a heavily loaded CI machine must
    // not turn scheduler starvation into a false test failure.
    support::wait_for(30, || {
        let claimed = st.claimed();
        claimed.contains(&task.task_id).then_some(())
    })
    .await;

    // Arm the cancel instruction; the next busy heartbeater tick delivers it.
    st.request_cancel(&task.task_id);

    // Barrier 2: terminal status settles on `cancelled` (never done/error).
    let status = support::wait_for(120, || st.status_of(&task.task_id)).await;
    assert_eq!(status, "cancelled", "statuses={:?}", st.statuses());
    let (_, _, err) = st
        .statuses()
        .into_iter()
        .find(|(id, _, _)| id == &task.task_id)
        .unwrap();
    assert!(err.is_none(), "cancellation carries no error payload");

    // The one and only report; no stray error/done after cancellation.
    assert_eq!(
        st.statuses()
            .iter()
            .filter(|(id, _, _)| id == &task.task_id)
            .count(),
        1,
        "exactly one terminal report"
    );

    runner.abort();
    // Teardown: release the hung LLM stream so nothing lingers.
    notify.notify_one();
}
