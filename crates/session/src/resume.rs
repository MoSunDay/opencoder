//! Session recovery: reconstruct a `SessionState` from a durable store, and
//! cheap background title generation (uses `small_model`).

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context, Result};
use opencoder_core::{resolve_agent, Agent, Config, ContentBlock, Message, Role};
use opencoder_llm::{lower_messages, ChatRequest, ChatStream, LlmEvent};
use opencoder_store::{Store, SubagentStatus};

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

    let mut messages: Vec<Message> = store.load_messages(id).await?;

    // Reconcile subagent tasks stuck in Running state — the process was
    // interrupted mid-subagent. Mark them as Failed with an interrupted marker.
    let tasks = store.list_subagent_tasks(id).await.unwrap_or_default();
    for task in &tasks {
        if task.status == SubagentStatus::Running {
            tracing::warn!(task_id = %task.task_id, "marking stuck Running subagent as Failed on resume");
            let _ = store
                .complete_subagent_task(&task.task_id, "(interrupted)", false)
                .await;
        }
    }

    // If compaction was persisted, trim the summarized head and prepend
    // the summary message so the resumed transcript matches the pre-exit state.
    if let Some(skip) = meta.summary_seq {
        if skip > 0 {
            let skip = skip as usize;
            if skip < messages.len() {
                messages = messages[skip..].to_vec();
            } else {
                messages = Vec::new();
            }
        }
        if let Some(summary_text) = &meta.summary {
            let summary_msg = crate::compaction::compaction_message(summary_text.clone());
            messages.insert(0, summary_msg);
        }
    }

    // Reconcile dangling tool_use blocks. If the process was hard-interrupted
    // after the assistant requested tool calls but before the matching
    // tool_result messages were persisted, the transcript holds unmatched
    // `tool_use` ids -- which most OpenAI-compatible providers reject with
    // HTTP 400 on the next call. Synthesize error results for every dangling
    // call, persist them, and append them so history stays well-formed.
    let answered: HashSet<&str> = messages
        .iter()
        .flat_map(|m| m.blocks.iter())
        .filter_map(|b| match b {
            ContentBlock::ToolResult { tool_use_id, .. } => Some(tool_use_id.as_str()),
            _ => None,
        })
        .collect();
    let dangling: Vec<ContentBlock> = messages
        .iter()
        .flat_map(|m| m.blocks.iter())
        .filter_map(|b| match b {
            ContentBlock::ToolUse { id, .. } if !answered.contains(id.as_str()) => {
                Some(ContentBlock::ToolResult {
                    tool_use_id: id.clone(),
                    content: "session interrupted: tool result missing".to_string(),
                    is_error: true,
                })
            }
            _ => None,
        })
        .collect();
    if !dangling.is_empty() {
        let n_dangling = dangling.len();
        let synthetic = Message {
            id: crate::runner::new_id(),
            role: Role::Tool,
            blocks: dangling,
            model: None,
            agent: None,
            usage: opencoder_core::MessageUsage::default(),
            created_at: opencoder_core::message::now_ms(),
            synthetic: true,
        };
        tracing::warn!(
            session_id = id,
            count = n_dangling,
            "synthesizing error tool_result for dangling tool_use on resume"
        );
        // Persist so a subsequent resume sees a well-formed transcript.
        let _ = store.append_message(id, &synthetic).await;
        messages.push(synthetic);
    }

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
        last_usage: opencoder_llm::Usage::default(),
        store: Some(store),
        skill_prompt: Arc::new(Mutex::new(None)),
        persisted_count: n,
        session_created: true,
        cancel: None,
        summary: meta.summary,
        summary_seq: meta.summary_seq,
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
        reasoning_effort: None,
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
            &opencoder_store::SessionPatch {
                title: Some(title),
                updated_at: Some(opencoder_core::message::now_ms()),
                ..Default::default()
            },
        )
        .await?;
    Ok(())
}

#[allow(dead_code)]
fn _ensure_agent_used(_a: &Agent) {}
