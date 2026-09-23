use anyhow::{ensure, Context, Result};
use opencoder_core::brain::layered::LayeredContext;
use opencoder_core::brain::layered::{LayeredDecision, LAYERED_SCHEMA_VERSION};
use opencoder_llm::{ChatRequest, ChatStream, LlmEvent, Message, RequestPurpose};

/// One layer decision. The model never sees child bodies: only the bounded
/// context assembled by [`super::layer_context`].
pub async fn activate(
    context: &LayeredContext,
    client: &dyn ChatStream,
    model: &str,
) -> Result<LayeredDecision> {
    ensure!(
        context.schema_version == LAYERED_SCHEMA_VERSION,
        "layer context is not a schema 5 request"
    );
    let mut stream = client.chat_stream(ChatRequest {
        purpose: RequestPurpose::Planning,
        model: model.into(),
        messages: vec![
            Message::system("layered-contract", super::prompt::PROMPT),
            Message::user("layered-context", super::prompt::instruction(context)?),
        ],
        tools: vec![],
        tool_choice: None,
        temperature: Some(0.0),
        max_tokens: Some(16384),
        reasoning_effort: None,
        cache_salt: None,
    })?;
    while let Some(event) = stream.recv().await {
        match event {
            LlmEvent::Completed { text, .. } => {
                ensure!(text.len() <= 256 * 1024, "layered decision exceeds 256 KiB");
                return serde_json::from_str(text.trim())
                    .context("layered decision must be a strict JSON object");
            }
            LlmEvent::Error(error) => anyhow::bail!("layered provider: {error}"),
            _ => {}
        }
    }
    anyhow::bail!("layered stream ended without completion")
}
