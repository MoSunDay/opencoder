use std::collections::HashMap;

use anyhow::{anyhow, Result};
use opencoder_core::{MessageUsage, ToolArc};
use opencoder_llm::tool_call::CompletedToolCall;
use opencoder_llm::{lower_messages, ChatRequest, ChatStream, LlmEvent, Usage};

use crate::prompt::build_system;
use crate::tools::schema_for;
use crate::SessionState;

use super::event::SessionEvent;
use super::steer::{await_cancel, await_turn_cancel};

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
    let mut turn_cancel_fut = std::pin::pin!(await_turn_cancel(session));
    loop {
        // The event-level idle watchdog now lives inside the streaming client
        // (`ChatClient::run_stream`), which retries stalls before they ever
        // reach this loop. Only session-semantic interrupts remain here: a
        // double-Esc / web interrupt (cancel) and a subagent submit-now steer
        // (turn_cancel).
        tokio::select! {
            biased;
            _ = &mut cancel_fut => {
                on_event(SessionEvent::Status("interrupted".into()));
                return Ok((String::new(), String::new(), Vec::new(), None));
            }
            _ = &mut turn_cancel_fut => {
                // Turn interrupted by subagent steer "submit-now": return an
                // empty turn. The caller (run_loop) detects this via
                // is_turn_cancelled and continues the loop to absorb pending
                // steers — it does NOT break like a real cancel.
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
                        // Mid-stream retry: the client discarded its partial
                        // response and is regenerating from scratch. Drop any
                        // reasoning deltas accumulated this attempt so they
                        // aren't stitched onto the fresh frame.
                        reasoning_buf.clear();
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use anyhow::Result;
    use tokio::sync::mpsc;

    use opencoder_core::{resolve_agent, Config, ToolArc};
    use opencoder_llm::{ChatRequest, ChatStream, LlmEvent, MockChatClient};

    use crate::SessionState;

    use super::run_one_llm_call;

    fn make_session(client: Arc<dyn ChatStream>) -> SessionState {
        let cfg = Config::default();
        SessionState::new(
            "test",
            resolve_agent("act").unwrap(),
            cfg,
            client,
            std::env::temp_dir().join("opencoder-idle-test"),
        )
    }

    #[tokio::test]
    async fn busy_stream_completes() {
        let mock = MockChatClient::new().push_script(vec![
            LlmEvent::TextDelta("hello".into()),
            LlmEvent::Completed {
                text: "hello".into(),
                tool_calls: vec![],
                usage: None,
            },
        ]);
        let session = make_session(Arc::new(mock) as Arc<dyn ChatStream>);
        let registry: HashMap<String, ToolArc> = HashMap::new();
        let result = run_one_llm_call(&session, &registry, &mut |_| {}).await;
        assert!(result.is_ok(), "expected success, got: {:?}", result);
        let (text, _, _, _) = result.unwrap();
        assert_eq!(text, "hello");
    }

    /// A custom `ChatStream` double that emits a fixed event script. Lets us
    /// drive the consumer loop with a synthetic mid-stream `Retrying` event,
    /// which the real `ChatClient` produces when it restarts an interrupted
    /// stream. The mock client cannot express this (it has no notion of
    /// retries), so a dedicated double is needed.
    struct ScriptedClient {
        events: std::sync::Mutex<Option<Vec<LlmEvent>>>,
    }

    impl ChatStream for ScriptedClient {
        fn chat_stream(&self, _req: ChatRequest) -> Result<mpsc::Receiver<LlmEvent>> {
            let events = self
                .events
                .lock()
                .unwrap()
                .take()
                .expect("ScriptedClient used more than once");
            let (tx, rx) = mpsc::channel::<LlmEvent>(128);
            tokio::spawn(async move {
                for ev in events {
                    tokio::task::yield_now().await;
                    if tx.send(ev).await.is_err() {
                        break;
                    }
                }
            });
            Ok(rx)
        }
        fn backend(&self) -> &'static str {
            "scripted"
        }
    }

    /// A mid-stream `Retrying` must discard deltas accumulated so far. The
    /// double streams some partial text + reasoning, signals a retry, then
    /// streams a fresh frame ending in `Completed`. The returned text must be
    /// the FINAL frame's text (never stitched), and reasoning from the dead
    /// first attempt must be cleared.
    #[tokio::test]
    async fn mid_stream_retry_clears_accumulated_state() {
        let double = ScriptedClient {
            events: std::sync::Mutex::new(Some(vec![
                LlmEvent::TextDelta("partial ".into()),
                LlmEvent::ReasoningDelta("dead-thought".into()),
                LlmEvent::Retrying {
                    attempt: 1,
                    max: 3,
                },
                LlmEvent::TextDelta("final".into()),
                LlmEvent::ReasoningDelta("live-thought".into()),
                LlmEvent::Completed {
                    text: "final".into(),
                    tool_calls: vec![],
                    usage: None,
                },
            ])),
        };
        let session = make_session(Arc::new(double) as Arc<dyn ChatStream>);
        let registry: HashMap<String, ToolArc> = HashMap::new();
        let mut saw_retry = false;
        let result = run_one_llm_call(&session, &registry, &mut |ev| {
            if let crate::runner::SessionEvent::Status(s) = &ev {
                if s.contains("retry") {
                    saw_retry = true;
                }
            }
        })
        .await;
        assert!(result.is_ok(), "expected success, got: {:?}", result);
        let (text, reasoning, _, _) = result.unwrap();
        // Text always comes from the final Completed frame — never stitched
        // across the two attempts.
        assert_eq!(text, "final");
        // Reasoning from the dead first attempt is cleared on retry; only the
        // live frame's reasoning survives.
        assert_eq!(reasoning, "live-thought");
        assert!(saw_retry, "consumer should surface the retry status");
    }
}
