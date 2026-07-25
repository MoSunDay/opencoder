use std::collections::HashMap;

use anyhow::{anyhow, Result};
use opencoder_core::{MessageUsage, ToolArc};
use opencoder_llm::tool_call::CompletedToolCall;
use opencoder_llm::{lower_messages, ChatRequest, ChatStream, LlmEvent, Usage};

use crate::prompt::build_system;
use crate::tools::schema_for;
use crate::SessionState;

use super::event::SessionEvent;
use super::steer::await_cancel;

pub(super) async fn run_one_llm_call(
    session: &SessionState,
    registry: &HashMap<String, ToolArc>,
    on_event: &mut (impl FnMut(SessionEvent) + Send + ?Sized),
) -> Result<(String, String, Vec<CompletedToolCall>, Option<Usage>)> {
    let system = build_system(
        &session.agent,
        &session.working_dir,
        session.skill_prompt_cloned().as_deref(),
        &session.config.capabilities,
    );
    let mut to_send = vec![system];
    to_send.extend(session.messages.iter().cloned());
    let openai_msgs = lower_messages(&to_send);

    let skill_body = session.skill_prompt_cloned();
    let unlocked = crate::tools::latent::unlocked_from_body(skill_body.as_deref());
    let allowed: HashMap<String, ToolArc> = registry
        .iter()
        .filter(|(name, _)| {
            session.agent.tools.allows(name)
                && session.config.capabilities.tool_enabled(name)
                && (!crate::tools::latent::is_latent_tool(name.as_str())
                    || unlocked.contains(name.as_str()))
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let tool_schemas = schema_for(&allowed, session.agent.kind, &session.config.capabilities);

    let req = ChatRequest {
        model: session.model.clone(),
        messages: openai_msgs,
        tools: tool_schemas,
        tool_choice: if allowed.is_empty() {
            None
        } else {
            Some("auto".into())
        },
        temperature: None,
        max_tokens: session.config.max_tokens,
        reasoning_effort: session.config.reasoning_effort.clone(),
        cache_salt: crate::cache_salt_for(session),
    };
    let mut rx = session.client.chat_stream(req)?;
    let mut completed: Option<(String, Vec<CompletedToolCall>, Option<Usage>)> = None;
    let mut reasoning_buf = String::new();
    // True once a `Retrying` status has been shown; cleared (with an empty
    // Status event) the moment real content streams so the "↻ retry" badge
    // doesn't linger after recovery.
    let mut retried = false;
    let mut cancel_fut = std::pin::pin!(await_cancel(session));
    loop {
        tokio::select! {
            biased;
            _ = &mut cancel_fut => {
                on_event(SessionEvent::Status("interrupted".into()));
                return Ok((String::new(), String::new(), Vec::new(), None));
            }
            ev = rx.recv() => {
                let ev = match ev { Some(ev) => ev, None => break };
                match ev {
                    LlmEvent::TextDelta(t) => {
                        if retried {
                            retried = false;
                            on_event(SessionEvent::Status(String::new()));
                        }
                        on_event(SessionEvent::TextDelta(t));
                    }
                    LlmEvent::ReasoningDelta(r) => {
                        if retried {
                            retried = false;
                            on_event(SessionEvent::Status(String::new()));
                        }
                        reasoning_buf.push_str(&r);
                        on_event(SessionEvent::ReasoningDelta(r));
                    }
                    LlmEvent::ToolCallStart { .. } | LlmEvent::ToolCallDelta { .. } => {}
                    LlmEvent::Completed { text, tool_calls, usage } => {
                        if retried {
                            retried = false;
                            on_event(SessionEvent::Status(String::new()));
                        }
                        completed = Some((text, tool_calls, usage));
                    }
                    LlmEvent::Retrying { attempt, max } => {
                        retried = true;
                        on_event(SessionEvent::Status(format!(
                            "\u{21bb} retry {attempt}/{max}"
                        )));
                    }
                    LlmEvent::Error(e) => return Err(anyhow!(e)),
                }
            }
        }
    }
    let (text, tool_calls, usage) =
        completed.ok_or_else(|| anyhow!("stream ended without completion"))?;
    Ok((text, reasoning_buf, tool_calls, usage))
}

pub(super) fn core_usage(u: &Usage) -> MessageUsage {
    MessageUsage {
        input_tokens: u.input_tokens,
        output_tokens: u.output_tokens,
        total_tokens: u.total_tokens,
        cache_read_tokens: u.cache_read_tokens,
        cache_creation_tokens: u.cache_creation_tokens,
    }
}
