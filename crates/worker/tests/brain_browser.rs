#![cfg(not(windows))]
#[path = "scheduler_v4/client.rs"]
mod client;
#[path = "support/mod.rs"]
mod support;
use opencoder_llm::{ChatRequest, ChatStream, LlmEvent, MockChatClient};
use std::sync::Arc;

/// Capability CRUD uses embeddings; only model transports are replaced here.
struct BrowserModel(client::LayeredClient);

impl ChatStream for BrowserModel {
    fn chat_stream(
        &self,
        request: ChatRequest,
    ) -> anyhow::Result<tokio::sync::mpsc::Receiver<LlmEvent>> {
        self.0.chat_stream(request)
    }

    fn embed(&self, texts: &[String], model: &str) -> anyhow::Result<Vec<Vec<f32>>> {
        MockChatClient::new().embed(texts, model)
    }
}

/// Runs Chromium against real control/node HTTP and the durable scheduler.
/// Only the model transport is deterministic; browser network is not intercepted.
#[tokio::test]
#[ignore = "requires Chromium and the built SPA; run explicitly for browser acceptance"]
async fn schema_seven_canvas_parallel_return_and_execution_detail() {
    let fleet = support::Fleet::new_with_ui(
        1,
        Arc::new(BrowserModel(client::LayeredClient::reflecting_once())),
        true,
    )
    .await;
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
