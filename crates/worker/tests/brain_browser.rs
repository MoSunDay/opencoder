#[path = "support/brain.rs"]
mod graph_support;
#[path = "support/mod.rs"]
mod support;
use std::sync::Arc;

/// Runs Chromium against real control/node HTTP and the durable graph kernel.
/// Only the model transport is deterministic; browser network is not intercepted.
#[tokio::test]
#[ignore = "requires Chromium and the built SPA; run explicitly for browser acceptance"]
async fn edit_publish_and_run_graph_in_browser() {
    let fleet =
        support::Fleet::new_with_ui(1, Arc::new(graph_support::GraphClient::default()), true).await;
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
