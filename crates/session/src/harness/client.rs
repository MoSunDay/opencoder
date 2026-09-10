//! Delay native provider construction until it is actually needed. Codex never
//! calls this client, while mixed workflows still validate native requests.
use anyhow::Result;
use opencoder_core::Config;
use opencoder_llm::{ChatClient, ChatRequest, ChatStream, LlmEvent};
use std::sync::Arc;

pub fn configured_client(config: Config) -> Arc<dyn ChatStream> {
    Arc::new(ConfiguredClient(config))
}
struct ConfiguredClient(Config);
impl ChatStream for ConfiguredClient {
    fn request_body(&self, request: &ChatRequest) -> Result<serde_json::Value> {
        let ep = self.0.resolve_endpoint()?;
        ChatClient::from_config(&self.0, &ep)?.request_body(request)
    }
    fn chat_stream(&self, request: ChatRequest) -> Result<tokio::sync::mpsc::Receiver<LlmEvent>> {
        let ep = self.0.resolve_endpoint()?;
        ChatClient::from_config(&self.0, &ep)?.chat_stream(request)
    }
}
