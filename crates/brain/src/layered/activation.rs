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
        "layer context is not a schema 6 request"
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
                return parse_decision(&text)
                    .map_err(|error| anyhow::anyhow!("invalid layered decision: {error:#}"));
            }
            LlmEvent::Error(error) => anyhow::bail!("layered provider: {error}"),
            _ => {}
        }
    }
    anyhow::bail!("layered stream ended without completion")
}

/// Accept one JSON document, optionally wrapped in a single explicit JSON fence.
/// Never extract a substring from prose or repair an invalid decision.
fn parse_decision(text: &str) -> Result<LayeredDecision> {
    ensure!(text.len() <= 256 * 1024, "layered decision exceeds 256 KiB");
    let text = text.trim();
    let document = if let Some(body) = text
        .strip_prefix("```json\n")
        .or_else(|| text.strip_prefix("```json\r\n"))
    {
        body.strip_suffix("\n```")
            .context("layered decision JSON fence is not closed")?
            .trim()
    } else {
        text
    };
    serde_json::from_str(document).context("layered decision must contain one strict JSON object")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_json_fence_preserves_the_exact_decision() {
        let raw = r#"{"decision":"block","reason":"missing release input"}"#;
        let plain = parse_decision(raw).unwrap();
        for fenced in [
            format!("```json\n{raw}\n```"),
            format!("```json\r\n{raw}\r\n```\n"),
        ] {
            assert_eq!(
                serde_json::to_value(parse_decision(&fenced).unwrap()).unwrap(),
                serde_json::to_value(&plain).unwrap()
            );
        }
    }

    #[test]
    fn prose_multiple_documents_and_broken_decisions_are_rejected() {
        for text in [
            "Here is the decision: {}",
            "```json\n{}\n```\nextra prose",
            "```json\n{}\n```\n```json\n{}\n```",
            "```json\n{\n```",
            "```json\n{}",
            "{} {}",
            "```json\n{\"decision\":\"invented\"}\n```",
        ] {
            assert!(parse_decision(text).is_err(), "accepted {text}");
        }
    }
}
