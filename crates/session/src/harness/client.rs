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
    fn chat_stream(&self, request: ChatRequest) -> Result<tokio::sync::mpsc::Receiver<LlmEvent>> {
        let ep = self.0.resolve_endpoint()?;
        ChatClient::new_with_read_timeout(
            &ep.base_url,
            &ep.api_key,
            &ep.headers,
            self.0.stream_idle_timeout(),
            self.0.network.proxy.as_deref(),
        )?
        .chat_stream(request)
    }
}
