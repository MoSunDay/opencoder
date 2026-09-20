#[path = "scheduler_browser/fixture.rs"]
mod browser_fixture;
#[path = "support/mod.rs"]
mod support;
use std::sync::Arc;

/// Runs Chromium against real control/node HTTP and the durable scheduler.
/// Only the model transport is deterministic; browser network is not intercepted.
#[tokio::test]
#[ignore = "requires Chromium and the built SPA; run explicitly for browser acceptance"]
async fn save_and_run_scheduler_with_five_execution_panels_in_browser() {
    let fleet =
        support::Fleet::new_with_ui(1, Arc::new(browser_fixture::BrowserClient), true).await;
    browser_fixture::seed(&fleet).await;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(300),
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
