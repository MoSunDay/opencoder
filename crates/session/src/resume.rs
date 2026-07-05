//! Session recovery: reconstruct a `SessionState` from a durable store, and
//! cheap background title generation (uses `small_model`).

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use opencode_core::{resolve_agent, Agent, Config, Message};
use opencode_llm::{lower_messages, ChatRequest, ChatStream, LlmEvent};
use opencode_store::Store;

use crate::SessionState;

/// Rebuild a session from persisted history. The agent/model come from the
/// stored session metadata when available, so a resumed session keeps its
/// original configuration rather than the caller's defaults.
pub async fn resume(
    store: Arc<dyn Store>,
    id: &str,
    mut config: Config,
    client: Arc<dyn ChatStream>,
    working_dir: PathBuf,
) -> Result<SessionState> {
    let meta = store
        .get_session(id)
        .await?
        .ok_or_else(|| anyhow!("session not found: {id}"))?;

    // Prefer the stored model/agent so resume is faithful to the original run.
    if let Some(m) = &meta.model {
        config.model = m.clone();
    }
    let agent_name = meta.agent.as_deref().unwrap_or(&config.agent.default);
    let agent = resolve_agent(agent_name)
        .or_else(|| resolve_agent("act"))
        .ok_or_else(|| anyhow!("agent not found: {agent_name}"))?;

    let messages: Vec<Message> = store.load_messages(id).await?;
    let n = messages.len();
    let model = config.model_id().to_string();

    let mut s = SessionState {
        id: id.to_string(),
        messages,
        agent,
        model,
        working_dir,
        config,
        client,
        step: 0,
        last_usage: opencode_llm::Usage::default(),
        store: Some(store),
        skill_prompt: None,
        persisted_count: n,
        session_created: true,
        cancel: None,
    };
    let _ = &mut s;
    Ok(s)
}

/// Generate a short title from the first user/assistant exchange, using the
/// small model when configured. Persists the title to the store. Non-fatal:
/// errors are logged and swallowed.
pub async fn generate_title(session: &SessionState) {
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
            &opencode_store::SessionPatch {
                title: Some(title),
                updated_at: Some(opencode_core::message::now_ms()),
                ..Default::default()
            },
        )
        .await?;
    Ok(())
}

#[allow(dead_code)]
fn _ensure_agent_used(_a: &Agent) {}
