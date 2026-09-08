//! Native title generation, kept outside the session recovery driver.
use crate::SessionState;
use anyhow::{anyhow, Context, Result};
use opencoder_llm::{lower_messages, ChatRequest, LlmEvent};
use opencoder_store::Store;
use std::sync::Arc;

/// Generate a short title from the first user/assistant exchange, using the
/// small model when configured. Persists the title to the store. Non-fatal:
/// errors are logged and swallowed.
pub async fn generate_title(session: &SessionState) {
    if session.harness.harness == opencoder_core::harness::Harness::Codex {
        return;
    }
    if session.store.is_none() {
        return;
    }
    let store = session.store.clone().unwrap();
    if let Err(e) = generate_title_inner(session, &store).await {
        tracing::warn!(session_id = %session.id, error = %e, "title generation failed");
    }
}

async fn generate_title_inner(session: &SessionState, store: &Arc<dyn Store>) -> Result<()> {
    let msgs = lower_messages(&session.messages);
    let req = ChatRequest {
        model: session.config.small_model_or_primary().to_string(),
        messages: msgs,
        tools: Vec::new(),
        tool_choice: None,
        temperature: Some(0.3),
        max_tokens: Some(64),
        reasoning_effort: None,
        cache_salt: crate::cache_salt_for(session),
    };
    let mut rx = session.client.chat_stream(req).context("title llm call")?;
    let mut text = String::new();
    while let Some(ev) = rx.recv().await {
        match ev {
            LlmEvent::TextDelta(t) => text.push_str(&t),
            LlmEvent::Completed { text: t, .. } => {
                if !t.is_empty() {
                    text = t;
                }
                break;
            }
            LlmEvent::Retrying { .. } => {
                // Mid-stream retry: drop deltas so the two attempts aren't
                // concatenated; the final `Completed` overwrites `text`.
                text.clear();
            }
            LlmEvent::Error(e) => return Err(anyhow!(e)),
            _ => {}
        }
    }
    let title: String = text.trim().chars().take(80).collect();
    if title.is_empty() {
        return Ok(());
    }
    store
        .update_session(
            &session.id,
            &opencoder_store::SessionPatch {
                title: Some(title),
                updated_at: Some(opencoder_core::message::now_ms()),
                ..Default::default()
            },
        )
        .await?;
    Ok(())
}
