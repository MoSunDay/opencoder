#[path = "support/scheduler.rs"]
mod scheduler_support;
#[path = "support/mod.rs"]
mod support;
use std::sync::Arc;
use support::Fleet;

/// Runs Chromium against real control/node HTTP and the v3 scheduler.
/// Only the model transport is deterministic; browser network is not intercepted.
#[tokio::test]
#[ignore = "requires Chromium and the built SPA; run explicitly for browser acceptance"]
async fn create_v3_run_and_open_indexed_execution_in_browser() {
    let fleet = Fleet::new_with_ui(
        1,
        Arc::new(scheduler_support::SchedulerClient::default()),
        true,
    )
    .await;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(180),
        tokio::process::Command::new("node")
            .arg(root.join("scripts/acceptance/brain/runtime.js"))
            .arg(&fleet.url)
            .kill_on_drop(true)
            .status(),
    )
    .await
    .expect("browser acceptance timed out")
    .unwrap();
    fleet.shutdown().await;
    assert!(result.success(), "browser acceptance failed");
}
